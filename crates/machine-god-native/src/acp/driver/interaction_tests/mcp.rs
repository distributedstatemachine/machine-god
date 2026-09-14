//! Explicit loopback HTTP exercises real ephemeral startup, not fake readiness.
use super::*;
use crate::mcp::{
    http::{McpHttpClock, tests::request as read_request},
    network::{McpResolverConfig, NativeMcpNetwork},
};
use machine_god_core::{ContentBlock, SessionId, SessionStore};
use std::{net::Ipv4Addr, time::Instant};
use tokio::{io::AsyncWriteExt, net::TcpListener};

mod command_tests;

struct Clock;
impl McpHttpClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}
pub(super) fn network() -> Arc<NativeMcpNetwork> {
    Arc::new(
        NativeMcpNetwork::new(
            McpResolverConfig::literal_only(),
            [9; 32],
            None,
            Arc::new(Clock),
            CancellationToken::new(),
            2,
        )
        .unwrap(),
    )
}

struct Server {
    listener: TcpListener,
    url_mode: bool,
    resource_mode: bool,
    stop: CancellationToken,
    continued: CancellationToken,
    release: CancellationToken,
    requests: Arc<Mutex<Vec<Value>>>,
}
impl Server {
    async fn new(url_mode: bool) -> Self {
        Self {
            listener: TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap(),
            url_mode,
            resource_mode: false,
            stop: CancellationToken::new(),
            continued: CancellationToken::new(),
            release: CancellationToken::new(),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
    async fn resource(url_mode: bool) -> Self {
        Self {
            resource_mode: true,
            ..Self::new(url_mode).await
        }
    }
    fn config(&self) -> Value {
        json!([{"name":"fixture","type":"http",
            "url":format!("http://127.0.0.1:{}/mcp", self.listener.local_addr().unwrap().port()),
            "headers":[]}])
    }
    async fn serve(&self) {
        let mut calls = 0usize;
        loop {
            let exchange = async {
                let (mut socket, _) = self.listener.accept().await.unwrap();
                let bytes = read_request(&mut socket).await;
                let body = memchr::memmem::find(&bytes, b"\r\n\r\n").unwrap() + 4;
                let request: Value = machine_god_core::json::from_slice(&bytes[body..]).unwrap();
                {
                    let mut requests = self.requests.lock().unwrap();
                    assert!(requests.len() < 8, "bounded fixture exchange count");
                    requests.push(request.clone());
                }
                let result = match request["method"].as_str().unwrap() {
                    "server/discover" => {
                        let capabilities = if self.resource_mode {
                            json!({"resources":{}})
                        } else {
                            json!({"tools":{}})
                        };
                        json!({"resultType":"complete","supportedVersions":["2026-07-28"],
                        "capabilities":capabilities})
                    }
                    "tools/list" => json!({"resultType":"complete","ttlMs":60000,"tools":[
                        {"name":"lookup","inputSchema":{"type":"object","properties":{}},
                         "annotations":{"readOnlyHint":true}}]}),
                    "resources/list" => json!({"resultType":"complete","ttlMs":60000,
                        "resources":[{"uri":"test://fixed","name":"fixed","mimeType":"text/plain"}]}),
                    "resources/templates/list" => {
                        json!({"resultType":"complete","ttlMs":60000,"resourceTemplates":[]})
                    }
                    method @ ("tools/call" | "resources/read") => {
                        assert_eq!(
                            method,
                            if self.resource_mode {
                                "resources/read"
                            } else {
                                "tools/call"
                            }
                        );
                        if self.resource_mode {
                            assert_eq!(request["params"]["uri"], "test://fixed");
                        }
                        self.operation_result(&mut calls).await
                    }
                    method => panic!("unexpected modern MCP method {method}"),
                };
                let mut envelope = json!({"jsonrpc":"2.0","id":request["id"]});
                envelope["result"] = result;
                let body = serde_json::to_vec(&envelope).unwrap();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
                socket.flush().await.unwrap();
            };
            if matches!(
                futures_util::future::select(Box::pin(self.stop.cancelled()), Box::pin(exchange))
                    .await,
                futures_util::future::Either::Left(_)
            ) {
                return;
            }
        }
    }

    async fn operation_result(&self, calls: &mut usize) -> Value {
        *calls += 1;
        if *calls == 1 {
            let params = if self.url_mode {
                json!({"mode":"url","message":"Confirm fixture access","url":"https://example.test/connect"})
            } else {
                json!({"mode":"form","message":"Confirm fixture value","requestedSchema":{
                    "type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]}})
            };
            json!({"resultType":"input_required",
                "inputRequests":{"confirm":{"method":"elicitation/create","params":params}},
                "requestState":{"step":1}})
        } else {
            assert_eq!(*calls, 2, "no automatic operation replay");
            self.continued.cancel();
            self.release.cancelled().await;
            if self.resource_mode {
                json!({"resultType":"complete","contents":[{"uri":"test://fixed","mimeType":"text/plain","text":"human resource complete"}]})
            } else {
                json!({"resultType":"complete","content":[{"type":"text","text":"x".repeat(70000)}]})
            }
        }
    }
}

#[test]
fn modern_form_and_url_wire_roundtrips_follow_actual_ephemeral_tool_continuation() {
    run(async {
        for url_mode in [false, true] {
            let server = Server::new(url_mode).await;
            let client = scenario(&server);
            // Both futures are structurally owned and joined: no detached task,
            // subprocess, DNS lookup, provider credential or remote endpoint.
            Box::pin(futures_util::future::join(client, server.serve())).await;
        }
    });
}

async fn start(server: &Server) -> (Arc<Factory>, NativeAcpConnection, String, AcpId, Value) {
    let transport = Arc::new(Script::default());
    transport.calls([
        tool(
            "select",
            "mcp_select_tool",
            json!({"name":"mcp_fixture_lookup"}),
        ),
        tool("lookup", "mcp_fixture_lookup", json!({})),
        answer(),
    ]);
    let (factory, mut connection, session) = open(transport, Some(server.config())).await;
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
        "session/set_config_option",
        json!({"sessionId":session,"configId":"mode","value":"auto"}),
    );
    response(&mut connection, 3).await;
    prompt(&mut connection, &session, 4);
    let (rpc, params) = client_request(&mut connection, "elicitation/create").await;
    assert_eq!(params["sessionId"], session);
    assert_eq!(params["toolCallId"], "lookup");
    assert_eq!(params["mode"], if server.url_mode { "url" } else { "form" });
    (factory, connection, session, rpc, params)
}

async fn scenario(server: &Server) {
    let (factory, mut connection, session, rpc, params) = Box::pin(start(server)).await;
    let elicitation_id = params.get("elicitationId").cloned();
    assert_eq!(elicitation_id.is_some(), server.url_mode);
    let result = if server.url_mode {
        json!({"action":"accept"})
    } else {
        json!({"action":"accept","content":{"answer":"selected"}})
    };
    reply(&mut connection, rpc.clone(), result);
    // A URL answer starts the real continuation but is not completion. Keep
    // its HTTP response withheld while driving all actual native output.
    let mut continued = Box::pin(server.continued.cancelled());
    futures_util::future::poll_fn(|cx| {
        use std::future::Future;
        if let Poll::Ready(Some(bytes)) = connection.poll_output(cx, 200) {
            match decode_frame(&bytes).unwrap() {
                AcpMessage::Notification { method, .. } => assert_eq!(method, "session/update"),
                message => panic!("completion before native continuation: {message:?}"),
            }
            cx.waker().wake_by_ref();
        }
        continued.as_mut().poll(cx)
    })
    .await;
    reply(&mut connection, rpc.clone(), json!({"action":"cancel"}));
    assert!(
        connection
            .clients
            .reply(&rpc, Ok(json!({"action":"cancel"})))
            .is_err()
    );
    server.release.cancel();
    terminal_receipt(&mut connection, &session, elicitation_id, server.url_mode).await;
    {
        let sent = server.requests.lock().unwrap();
        let calls = sent
            .iter()
            .filter(|request| request["method"] == "tools/call")
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1]["params"]["requestState"], json!({"step":1}));
        assert!(calls[1].to_string().contains(if server.url_mode {
            "accept"
        } else {
            "selected"
        }));
    }
    shutdown(&mut connection).await;
    drop(factory);
    server.stop.cancel();
}

#[test]
fn cancellation_and_eof_retire_real_mcp_input_without_continuation_or_url_completion() {
    run(async {
        for url_mode in [false, true] {
            for eof in [false, true] {
                let server = Server::new(url_mode).await;
                let client = async {
                    let (_factory, mut connection, session, rpc, _) =
                        Box::pin(start(&server)).await;
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
                        // The response helper rejects elicitation/complete notices.
                        assert_eq!(
                            response(&mut connection, 4).await["stopReason"],
                            "cancelled"
                        );
                        shutdown(&mut connection).await;
                    }
                    reply(&mut connection, rpc, json!({"action":"accept"}));
                    while let Some(bytes) =
                        futures_util::future::poll_fn(|cx| connection.poll_output(cx, 200)).await
                    {
                        if let AcpMessage::Notification { method, .. } =
                            decode_frame(&bytes).unwrap()
                        {
                            assert_ne!(method, "elicitation/complete");
                        }
                    }
                    assert!(!server.continued.is_cancelled());
                    assert_eq!(
                        server
                            .requests
                            .lock()
                            .unwrap()
                            .iter()
                            .filter(|request| request["method"] == "tools/call")
                            .count(),
                        1
                    );
                    server.stop.cancel();
                };
                Box::pin(futures_util::future::join(client, server.serve())).await;
            }
        }
    });
}

async fn terminal_receipt(
    connection: &mut NativeAcpConnection,
    session: &str,
    elicitation_id: Option<Value>,
    url_mode: bool,
) {
    let mut completed_call = false;
    let mut notices = 0usize;
    for index in 0..128 {
        match next(connection).await {
            AcpMessage::Notification {
                method,
                params: Some(params),
            } if method == "session/update" => {
                if params["update"]["toolCallId"] == "lookup"
                    && params["update"]["status"] == "completed"
                {
                    assert_eq!(params["update"]["rawOutput"]["resultType"], "complete");
                    assert_eq!(
                        params["update"]["rawOutput"]["content"][0]["text"]
                            .as_str()
                            .unwrap()
                            .len(),
                        70_000
                    );
                    completed_call = true;
                }
            }
            AcpMessage::Notification {
                method,
                params: Some(params),
            } => {
                assert_eq!(method, "elicitation/complete");
                assert!(url_mode);
                assert!(
                    completed_call,
                    "archive/tool receipt precedes completion notice"
                );
                assert_eq!(params["sessionId"], session);
                assert_eq!(params.get("elicitationId"), elicitation_id.as_ref());
                assert_archived(connection, session).await;
                notices += 1;
            }
            AcpMessage::Response {
                id: Some(id),
                outcome,
            } => {
                assert_eq!(id, AcpId::Integer(4));
                assert_eq!(outcome.unwrap()["stopReason"], "end_turn");
                break;
            }
            message => panic!("unexpected continuation frame: {message:?}"),
        }
        assert!(index < 127, "bounded continuation output");
    }
    assert!(completed_call);
    assert_eq!(notices, usize::from(url_mode));
    assert_archived(connection, session).await;
}

async fn assert_archived(connection: &NativeAcpConnection, session: &str) {
    let host = connection.selection.current_host().unwrap();
    let record = host
        .session_lifecycle()
        .session_store()
        .load(SessionId::new(session).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(record.messages.iter().flat_map(|message| &message.content).any(|block|
        matches!(block, ContentBlock::ToolResult {call_id, output} if call_id.as_str() == "lookup" && output.content["type"] == "tool_result_archive")));
}
