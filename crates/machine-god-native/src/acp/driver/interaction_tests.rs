//! Actual ACP replies drive native tools; labels alone never grant execution.
use super::*;
use crate::acp::{protocol::decode_frame, selection::tests::fixture::Factory};
use crate::{AiGatewayByteStream, AiGatewayTransport, AiGatewayTransportRequest};
use machine_god_core::{BoxFuture, ProviderError};
use serde_json::json;
use std::{collections::VecDeque, sync::Mutex, time::Duration};

mod fixture;
#[cfg(feature = "mcp-http")]
mod mcp;
use fixture::*;

#[test]
fn permission_wire_options_preserve_call_provenance_and_grants_are_volatile() {
    run(async {
        for option in ["allow_once", "allow_always"] {
            let transport = Arc::new(Script::default());
            transport.calls([
                tool(
                    "first",
                    "write_file",
                    json!({"path":"note","content":"first"}),
                ),
                tool(
                    "second",
                    "write_file",
                    json!({"path":"note","content":"first"}),
                ),
                answer(),
            ]);
            let (factory, mut connection, session) = open(transport.clone(), None).await;
            // File grants bind both preimage and postimage, not merely a path.
            // Repeating this exact write keeps the grant identity unchanged.
            std::fs::write(factory.workspace.join("note"), b"first").unwrap();
            prompt(&mut connection, &session, 3);
            let (first, params) =
                client_request(&mut connection, "session/request_permission").await;
            assert_eq!(params["sessionId"], session);
            assert_eq!(params["toolCall"]["toolCallId"], "first");
            assert_eq!(
                params["toolCall"]["rawInput"],
                json!({"path":"note","content":"first"})
            );
            assert_eq!(params["options"].as_array().unwrap().len(), 3);
            reply(
                &mut connection,
                first.clone(),
                json!({"outcome":{"outcome":"selected","optionId":option}}),
            );
            if option == "allow_once" {
                let (second, params) =
                    client_request(&mut connection, "session/request_permission").await;
                assert_ne!(second, first);
                assert_eq!(params["toolCall"]["toolCallId"], "second");
                assert_eq!(
                    std::fs::read(factory.workspace.join("note")).unwrap(),
                    b"first"
                );
                reply(
                    &mut connection,
                    first.clone(),
                    json!({"outcome":{"outcome":"selected","optionId":"allow_always"}}),
                );
                assert_eq!(
                    std::fs::read(factory.workspace.join("note")).unwrap(),
                    b"first"
                );
                reply(
                    &mut connection,
                    second,
                    json!({"outcome":{"outcome":"selected","optionId":"allow_once"}}),
                );
            }
            assert_eq!(response(&mut connection, 3).await["stopReason"], "end_turn");
            assert_tool_outcomes(
                &connection,
                &session,
                &[("first", false), ("second", false)],
            )
            .await;
            assert_eq!(
                std::fs::read(factory.workspace.join("note")).unwrap(),
                b"first"
            );
            assert!(
                connection
                    .clients
                    .reply(&first, Ok(json!({"outcome":{"outcome":"cancelled"}})))
                    .is_err()
            );
            assert_volatile_after_load(&factory, &mut connection, &transport, &session).await;
            shutdown(&mut connection).await;
        }
    });
}

async fn assert_volatile_after_load(
    factory: &Factory,
    connection: &mut NativeAcpConnection,
    transport: &Script,
    session: &str,
) {
    request(
        connection,
        4,
        "session/load",
        json!({"sessionId":session,"cwd":factory.workspace}),
    );
    response(connection, 4).await;
    transport.calls([
        tool(
            "after-load",
            "write_file",
            json!({"path":"note","content":"first"}),
        ),
        answer(),
    ]);
    prompt(connection, session, 5);
    let (pending, params) = client_request(connection, "session/request_permission").await;
    assert_eq!(params["toolCall"]["toolCallId"], "after-load");
    reply(
        connection,
        pending,
        json!({"outcome":{"outcome":"selected","optionId":"reject_once"}}),
    );
    response(connection, 5).await;
    assert_tool_outcomes(connection, session, &[("after-load", true)]).await;
    assert_eq!(
        std::fs::read(factory.workspace.join("note")).unwrap(),
        b"first"
    );
}

#[test]
fn ordinary_question_wire_reply_uses_real_tool_source_and_rejects_duplicates() {
    run(async {
        let transport = Arc::new(Script::default());
        let questions = json!({"questions":[{"question":"Choose a route","options":[{"label":"A"},{"label":"B"}]}]});
        transport.calls([
            tool("question-one", "ask_user_question", questions.clone()),
            tool("question-two", "ask_user_question", questions),
            answer(),
        ]);
        let (_factory, mut connection, session) = open(transport.clone(), None).await;
        prompt(&mut connection, &session, 3);
        let (first, params) = client_request(&mut connection, "elicitation/create").await;
        assert_eq!(params["mode"], "form");
        assert_eq!(params["sessionId"], session);
        assert_eq!(params["toolCallId"], "question-one");
        assert_eq!(params["requestedSchema"]["required"], json!(["question_1"]));
        reply(
            &mut connection,
            first.clone(),
            json!({"action":"accept","content":{"question_1":"free-form"}}),
        );
        let (second, params) = client_request(&mut connection, "elicitation/create").await;
        assert_ne!(first, second);
        assert_eq!(params["toolCallId"], "question-two");
        reply(
            &mut connection,
            first.clone(),
            json!({"action":"accept","content":{"question_1":"stale"}}),
        );
        assert!(
            connection
                .clients
                .reply(&first, Ok(json!({"action":"cancel"})))
                .is_err()
        );
        reply(
            &mut connection,
            second,
            json!({"action":"accept","content":{"question_1":"fresh"}}),
        );
        response(&mut connection, 3).await;
        {
            let requests = transport.requests.lock().unwrap();
            let last = requests.last().unwrap().to_string();
            assert!(last.contains("free-form"));
            assert!(last.contains("fresh"));
            assert!(!last.contains("stale"));
        }
        shutdown(&mut connection).await;
    });
}

#[test]
fn cancellation_and_eof_retire_pending_permission_without_executing_it() {
    run(async {
        for eof in [false, true] {
            let transport = Arc::new(Script::default());
            transport.calls([
                tool(
                    "never-run",
                    "write_file",
                    json!({"path":"not-written","content":"no"}),
                ),
                answer(),
            ]);
            let (factory, mut connection, session) = open(transport, None).await;
            prompt(&mut connection, &session, 3);
            let (pending, _) = client_request(&mut connection, "session/request_permission").await;
            if eof {
                shutdown(&mut connection).await;
            } else {
                connection
                    .receive(
                        AcpMessage::Notification {
                            method: "session/cancel".into(),
                            params: Some(json!({"sessionId":session})),
                        },
                        200,
                    )
                    .unwrap();
                assert_eq!(
                    response(&mut connection, 3).await["stopReason"],
                    "cancelled"
                );
            }
            reply(
                &mut connection,
                pending,
                json!({"outcome":{"outcome":"selected","optionId":"allow_once"}}),
            );
            assert!(!factory.workspace.join("not-written").exists());
            shutdown(&mut connection).await;
        }
    });
}
