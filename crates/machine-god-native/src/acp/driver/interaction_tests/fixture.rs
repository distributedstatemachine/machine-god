use super::*;

#[derive(Default)]
pub(super) struct Script {
    responses: Mutex<VecDeque<Vec<u8>>>,
    pub requests: Mutex<Vec<Value>>,
}
impl Script {
    pub fn calls(&self, calls: impl IntoIterator<Item = Vec<u8>>) {
        self.responses.lock().unwrap().extend(calls);
    }
}
impl AiGatewayTransport for Script {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        Box::pin(async move {
            let request: Value = machine_god_core::json::from_slice(request.body()).unwrap();
            let review = request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "permission_decision");
            self.requests.lock().unwrap().push(request);
            let bytes = if review {
                tool(
                    "review",
                    "permission_decision",
                    json!({"risk":"low","authorization":"unknown",
                    "decision":"allow","rationale":"User requested development operation."}),
                )
            } else {
                self.responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(answer)
            };
            Ok(Box::pin(futures_util::stream::iter([Ok(bytes)])) as AiGatewayByteStream)
        })
    }
}
pub(super) fn tool(id: &str, name: &str, input: Value) -> Vec<u8> {
    let mut event = json!({"type":"tool-call","toolCallId":id,"toolName":name});
    event["input"] = input;
    format!("data: {event}\n\ndata: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"tool-calls\"}}}}\n\n").into_bytes()
}
pub(super) fn answer() -> Vec<u8> {
    b"data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"done\"}\n\ndata: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n".to_vec()
}
pub(super) fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(20), future)
                .await
                .expect("wire interaction must settle");
        });
}
pub(super) fn request(connection: &mut NativeAcpConnection, id: i64, method: &str, params: Value) {
    // Pass actual encoded bytes through the modern decoder before dispatch.
    let bytes = protocol::encode_frame(&AcpMessage::Request {
        id: AcpId::Integer(id),
        method: method.into(),
        params: Some(params),
    })
    .unwrap();
    connection
        .receive(decode_frame(&bytes).unwrap(), 200)
        .unwrap();
}
pub(super) fn reply(connection: &mut NativeAcpConnection, id: AcpId, result: Value) {
    let bytes = protocol::encode_frame(&AcpMessage::Response {
        id: Some(id),
        outcome: Ok(result),
    })
    .unwrap();
    connection
        .receive(decode_frame(&bytes).unwrap(), 200)
        .unwrap();
}
pub(super) async fn next(connection: &mut NativeAcpConnection) -> AcpMessage {
    let bytes = futures_util::future::poll_fn(|cx| connection.poll_output(cx, 200))
        .await
        .expect("connection retains output");
    decode_frame(&bytes).unwrap()
}
pub(super) async fn response(connection: &mut NativeAcpConnection, id: i64) -> Value {
    for _ in 0..128 {
        match next(connection).await {
            AcpMessage::Response {
                id: Some(actual),
                outcome,
            } => {
                assert_eq!(actual, AcpId::Integer(id));
                return outcome.unwrap();
            }
            AcpMessage::Notification { method, .. } => assert_eq!(method, "session/update"),
            message => panic!("unexpected frame while awaiting response: {message:?}"),
        }
    }
    panic!("bounded response");
}
pub(super) async fn client_request(
    connection: &mut NativeAcpConnection,
    method: &str,
) -> (AcpId, Value) {
    for _ in 0..128 {
        match next(connection).await {
            AcpMessage::Request {
                id,
                method: actual,
                params: Some(params),
            } => {
                assert_eq!(actual, method);
                return (id, params);
            }
            AcpMessage::Notification { method, params } => {
                assert_eq!(method, "session/update");
                if let Some(params) = params {
                    assert_ne!(
                        params["update"]["status"], "failed",
                        "fixture tool failed: {}",
                        params["update"]["rawOutput"]
                    );
                }
            }
            message => panic!("unexpected frame while awaiting native human input: {message:?}"),
        }
    }
    panic!("bounded client request");
}
pub(super) async fn open(
    transport: Arc<Script>,
    servers: Option<Value>,
) -> (Arc<Factory>, NativeAcpConnection, String) {
    let factory = Arc::new(Factory::new());
    let clients = NativeAcpClientRequests::new().unwrap();
    *factory.transport_override.lock().unwrap() = Some(transport);
    *factory.prompt_bridge_override.lock().unwrap() = Some(clients.bridge());
    *factory.presenter_override.lock().unwrap() = Some(clients.presenter());
    #[cfg(feature = "mcp-http")]
    if servers.is_some() {
        *factory.network_override.lock().unwrap() = Some(super::mcp::network());
    }
    let mut connection = NativeAcpConnection::new(factory.clone(), clients);
    request(
        &mut connection,
        1,
        "initialize",
        json!({"protocolVersion":1}),
    );
    response(&mut connection, 1).await;
    let mut params = json!({"cwd":factory.workspace});
    if let Some(servers) = servers {
        params["mcpServers"] = servers;
    }
    request(&mut connection, 2, "session/new", params);
    let id = response(&mut connection, 2).await["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    (factory, connection, id)
}
pub(super) fn prompt(connection: &mut NativeAcpConnection, session: &str, id: i64) {
    request(
        connection,
        id,
        "session/prompt",
        json!({"sessionId":session,"prompt":[{"type":"text","text":"Run the requested development operation"}]}),
    );
}
pub(super) async fn shutdown(connection: &mut NativeAcpConnection) {
    connection.begin_shutdown();
    futures_util::future::poll_fn(|cx| {
        let _ = connection.poll_progress(cx, 200);
        if connection.is_closed() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    assert!(connection.error().is_none(), "{:?}", connection.error());
}

pub(super) async fn assert_tool_outcomes(
    connection: &NativeAcpConnection,
    session: &str,
    expected: &[(&str, bool)],
) {
    use machine_god_core::{ContentBlock, SessionId, SessionStore};
    let record = connection
        .selection
        .current_host()
        .unwrap()
        .session_lifecycle()
        .session_store()
        .load(SessionId::new(session).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(
        !record
            .metadata
            .contains_key(crate::NATIVE_SESSION_PERMISSION_RULES_KEY),
        "wire grants must not create persistent saved permission rules"
    );
    for (id, is_error) in expected {
        let outcomes = record
            .messages
            .iter()
            .flat_map(|message| &message.content)
            .filter_map(|block| match block {
                ContentBlock::ToolResult { call_id, output } if call_id.as_str() == *id => {
                    Some(output)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].is_error, *is_error);
    }
}
