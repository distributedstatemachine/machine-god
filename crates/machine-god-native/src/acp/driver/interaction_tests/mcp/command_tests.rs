//! Human commands use real ephemeral resource catalogs, never model tool calls.
use super::*;

struct Human {
    factory: Arc<Factory>,
    transport: Arc<Script>,
    connection: Box<NativeAcpConnection>,
    session: String,
    rpc: AcpId,
    params: Value,
}

async fn start_human(server: &Server) -> Human {
    let transport = Arc::new(Script::default());
    let (factory, connection, session) =
        Box::pin(open(transport.clone(), Some(server.config()))).await;
    let mut connection = Box::new(connection);
    connection
        .selection
        .current_host()
        .unwrap()
        .mcp_ephemeral_owner()
        .unwrap()
        .ready()
        .unwrap();
    request(
        &mut connection,
        3,
        "session/prompt",
        json!({"sessionId":session,
        "prompt":[{"type":"text","text":"/mcp resource read fixture test://fixed"}]}),
    );
    let (rpc, params) = client_request(&mut connection, "elicitation/create").await;
    assert_eq!(params["sessionId"], session);
    assert_eq!(params["mode"], if server.url_mode { "url" } else { "form" });
    assert!(params.get("toolCallId").is_none());
    assert_eq!(params.get("elicitationId").is_some(), server.url_mode);
    assert!(connection.command.is_some());
    assert!(connection.prompt.is_none());
    assert!(transport.requests.lock().unwrap().is_empty());
    Human {
        factory,
        transport,
        connection,
        session,
        rpc,
        params,
    }
}

#[test]
fn human_resource_form_and_url_complete_after_real_continuation_before_prompt_reply() {
    run(async {
        for url_mode in [false, true] {
            let server = Server::resource(url_mode).await;
            let client = async {
                let mut human = Box::pin(start_human(&server)).await;
                let result = if url_mode {
                    json!({"action":"accept"})
                } else {
                    json!({"action":"accept","content":{"answer":"selected"}})
                };
                reply(&mut human.connection, human.rpc.clone(), result);
                wait_for_continuation(&mut human.connection, &server).await;
                // A duplicate wire response cannot cancel the owned continuation.
                reply(
                    &mut human.connection,
                    human.rpc.clone(),
                    json!({"action":"cancel"}),
                );
                server.release.cancel();
                command_response(&mut human, false).await;
                assert!(human.transport.requests.lock().unwrap().is_empty());
                {
                    let sent = server.requests.lock().unwrap();
                    let reads: Vec<_> = sent
                        .iter()
                        .filter(|request| request["method"] == "resources/read")
                        .collect();
                    assert_eq!(reads.len(), 2);
                    assert_eq!(reads[1]["params"]["requestState"], json!({"step":1}));
                    assert_eq!(
                        reads[1]["params"]["inputResponses"]["confirm"]["action"],
                        "accept"
                    );
                    if !url_mode {
                        assert_eq!(
                            reads[1]["params"]["inputResponses"]["confirm"]["content"]["answer"],
                            "selected"
                        );
                    }
                    assert!(!sent.iter().any(|request| request["method"] == "tools/call"));
                }
                shutdown(&mut human.connection).await;
                server.stop.cancel();
            };
            Box::pin(futures_util::future::join(client, server.serve())).await;
        }
    });
}

async fn wait_for_continuation(connection: &mut NativeAcpConnection, server: &Server) {
    let mut continued = Box::pin(server.continued.cancelled());
    futures_util::future::poll_fn(|cx| {
        use std::future::Future;
        if let Poll::Ready(Some(bytes)) = connection.poll_output(cx, 200) {
            match decode_frame(&bytes).unwrap() {
                AcpMessage::Notification {
                    method,
                    params: Some(params),
                } => {
                    assert_eq!(
                        method, "session/update",
                        "URL notice must await terminal response"
                    );
                    assert!(
                        params["update"].get("command_result").is_none(),
                        "command result must await terminal response"
                    );
                }
                message => panic!("command response before peer completion: {message:?}"),
            }
            cx.waker().wake_by_ref();
        }
        continued.as_mut().poll(cx)
    })
    .await;
}

async fn command_response(human: &mut Human, cancelled: bool) {
    let mut receipt_seen = false;
    let mut notices = 0usize;
    for _ in 0..32 {
        match next(&mut human.connection).await {
            AcpMessage::Notification {
                method,
                params: Some(params),
            } if method == "session/update" => {
                if let Some(receipt) = params["update"].get("command_result") {
                    assert!(!receipt_seen, "one command receipt");
                    assert_eq!(params["sessionId"], human.session);
                    assert!(params["update"].get("toolCallId").is_none());
                    assert_eq!(receipt["command"], "/mcp");
                    assert_eq!(receipt["kind"], "native_command");
                    assert_eq!(receipt["cancelled"], cancelled);
                    if !cancelled {
                        assert_eq!(receipt["status"], "completed");
                        assert_eq!(receipt["receipt"]["resultType"], "complete");
                        assert_eq!(
                            receipt["receipt"]["contents"][0]["text"],
                            "human resource complete"
                        );
                    }
                    receipt_seen = true;
                }
            }
            AcpMessage::Notification {
                method,
                params: Some(params),
            } => {
                assert_eq!(method, "elicitation/complete");
                assert!(!cancelled);
                assert_eq!(params["sessionId"], human.session);
                assert_eq!(
                    params.get("elicitationId"),
                    human.params.get("elicitationId")
                );
                assert!(human.params.get("elicitationId").is_some());
                assert!(params.get("toolCallId").is_none());
                notices += 1;
            }
            AcpMessage::Response {
                id: Some(id),
                outcome,
            } => {
                assert_eq!(
                    id,
                    AcpId::Integer(3),
                    "old command reply precedes replacement reply"
                );
                assert!(
                    receipt_seen,
                    "command_result precedes its prompt RPC response"
                );
                assert_eq!(
                    outcome.unwrap()["stopReason"],
                    if cancelled { "cancelled" } else { "end_turn" }
                );
                assert_eq!(
                    notices,
                    usize::from(!cancelled && human.params.get("elicitationId").is_some())
                );
                return;
            }
            message => panic!("unexpected human command output: {message:?}"),
        }
    }
    panic!("bounded human command output");
}

#[test]
fn human_command_cancellation_and_replacement_do_not_rebind_pending_answers() {
    run(async {
        for url_mode in [false, true] {
            for replace in [false, true] {
                let server = Server::resource(url_mode).await;
                let client = async {
                    let mut human = Box::pin(start_human(&server)).await;
                    if replace {
                        request(
                            &mut human.connection,
                            4,
                            "session/new",
                            json!({"cwd":human.factory.workspace}),
                        );
                    } else {
                        human
                            .connection
                            .receive(
                                AcpMessage::Notification {
                                    method: "session/cancel".into(),
                                    params: Some(json!({"sessionId":human.session})),
                                },
                                200,
                            )
                            .unwrap();
                    }
                    command_response(&mut human, true).await;
                    if replace {
                        let selected = response(&mut human.connection, 4).await;
                        assert_ne!(selected["sessionId"], human.session);
                    }
                    // After native retirement, even a previously valid accept
                    // cannot acquire the new principal or submit resources/read.
                    let late = if url_mode {
                        json!({"action":"accept"})
                    } else {
                        json!({"action":"accept","content":{"answer":"stale"}})
                    };
                    reply(&mut human.connection, human.rpc.clone(), late);
                    assert!(
                        human
                            .connection
                            .clients
                            .reply(&human.rpc, Ok(json!({"action":"cancel"})))
                            .is_err()
                    );
                    assert!(human.transport.requests.lock().unwrap().is_empty());
                    assert!(!server.continued.is_cancelled());
                    assert_eq!(
                        server
                            .requests
                            .lock()
                            .unwrap()
                            .iter()
                            .filter(|request| request["method"] == "resources/read")
                            .count(),
                        1
                    );
                    shutdown(&mut human.connection).await;
                    while let Some(bytes) =
                        futures_util::future::poll_fn(|cx| human.connection.poll_output(cx, 200))
                            .await
                    {
                        if let AcpMessage::Notification { method, .. } =
                            decode_frame(&bytes).unwrap()
                        {
                            assert_ne!(method, "elicitation/complete");
                        }
                    }
                    server.stop.cancel();
                };
                Box::pin(futures_util::future::join(client, server.serve())).await;
            }
        }
    });
}
