//! Full engine permission/archive routing over an actual modern HTTP producer.
use super::*;
use crate::mcp::{
    config::{McpServerConfig, McpTransportConfig},
    endpoint::McpEndpoint,
    headers::McpResolvedHeaders,
    http::{McpHttpClock, McpHttpDestination, tests::request},
    http_peer::{McpHttpPeer, McpHttpPeerOptions},
    lifetime::McpPeerLifetime,
    protocol::TransportKind,
};
use futures_util::future::join;
use std::net::Ipv4Addr;
use tokio::{io::AsyncWriteExt, net::TcpListener};

struct LiveClock;
impl McpHttpClock for LiveClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move { tokio::time::sleep_until(deadline.into()).await })
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

async fn respond(listener: &TcpListener, method: &str, body: &[u8], sse: bool) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let received = request(&mut socket).await;
    assert!(String::from_utf8_lossy(&received).contains(&format!("mcp-method: {method}")));
    let media = if sse {
        "text/event-stream"
    } else {
        "application/json"
    };
    socket
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    socket.write_all(body).await.unwrap();
    socket.flush().await.unwrap();
}

fn exercise_http(result_nodes: usize, notification_nodes: usize, calls: usize, sse: bool) {
    let archive = Archive::new();
    crate::mcp::http::tests::executor().block_on(async {
        tokio::time::timeout(Duration::from_secs(15), async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let url = format!("http://127.0.0.1:{}/mcp", address.port());
            let config = McpServerConfig::decode("calendar", &serde_json::to_vec(&json!({"type":"http","url":url})).unwrap()).unwrap();
            let McpTransportConfig::Http(remote) = config.transport() else { panic!() };
            let options = McpHttpPeerOptions {
                destination: McpHttpDestination::new(McpEndpoint::parse(&url).unwrap(), &[address]).unwrap(),
                trust: None,
                headers: McpResolvedHeaders::resolve(remote, |_| None, None, &[]).unwrap(),
                clock: Arc::new(LiveClock),
                transport: TransportKind::StreamableHttp,
                lifetime: McpPeerLifetime::OwnerControlled,
            };
            let result = json!({"resultType":"complete","content":[],"structuredContent":vec![Value::Null; result_nodes]});
            let expected = result.clone();
            let server = async {
                respond(&listener, "server/discover", br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}}}}"#, false).await;
                respond(&listener, "tools/list", br#"{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","ttlMs":600000,"tools":[{"name":"lookup","inputSchema":{"type":"object"}}]}}"#, false).await;
                for id in 3..3 + calls {
                    let response = json!({"jsonrpc":"2.0","id":id,"result":result});
                    let body = if sse {
                        let mut body = String::new();
                        for progress in 0..33 {
                            let notice = json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":id,"progress":progress,"extra":vec![Value::Null; notification_nodes]}});
                            body.push_str(&format!("data: {notice}\n\n"));
                            if notification_nodes > 0 { break; }
                        }
                        body.push_str(&format!("data: {response}\n\n"));
                        body.into_bytes()
                    } else { serde_json::to_vec(&response).unwrap() };
                    respond(&listener, "tools/call", &body, sse).await;
                }
            };
            let client = async {
                let mut peer = McpHttpPeer::connect(options, CancellationToken::new(), deadline()).await.unwrap();
                let epoch = Instant::now();
                let raw = peer.catalog(McpCatalogKind::Tools, McpCatalogLimits::default(), epoch, deadline()).await.unwrap();
                let catalog = McpDescriptorCatalog::admit(raw, McpDescriptorLimits::default()).unwrap();
                let fixture = Fixture::with_executor(
                    &vec![json!({}); calls], PermissionMode::Auto,
                    archive.executor.clone(), archive.executor.execution_policy(), false,
                    move |runtime, _| runtime.prepare_candidate(vec![NativeMcpServerCandidate {
                        server: Arc::from("calendar"), configuration: Arc::from(&b"config"[..]),
                        authentication: Arc::from(&b"auth"[..]), catalogs: vec![catalog], refresh: None,
                        catalog_epoch: epoch, peer: NativeMcpOwnedPeer::Http(Box::new(peer)),
                        operation_timeout: Duration::from_secs(5), authority_cancellations: Arc::from([]),
                    }], &[MCP_SELECT_TOOL_NAME]).unwrap(),
                );
                fixture.conversation.enqueue("Use the selected tool".into()).unwrap();
                let turn = fixture.conversation.start_next(1).await.unwrap().unwrap();
                let events = turn.collect::<Vec<_>>().await.into_iter().collect::<std::result::Result<Vec<_>, _>>().unwrap();
                let outputs = events.iter().filter_map(|event| match &event.payload {
                    TurnEvent::ToolFinished { call_id, output } if call_id.as_str().starts_with("call-") => Some(output),
                    _ => None,
                }).collect::<Vec<_>>();
                fixture.runtime.close();
                fixture.runtime.drain_retired(deadline(), CancellationToken::new()).await.unwrap();
                assert_eq!(outputs.len(), calls);
                if notification_nodes > 0 {
                    assert!(outputs[0].is_error);
                } else {
                    for output in outputs { assert_eq!(output, &ToolOutput::success(expected.clone())); }
                    if result_nodes > 65_536 {
                        let stored = persisted(&record(&fixture), "call-0");
                        assert_eq!(archive.read(&stored.content), ToolOutput::success(expected));
                    }
                }
            };
            join(client, server).await;
        }).await.unwrap();
    });
}

#[test]
fn http_tools_accept_codec_node_budget_for_json_and_sse_and_archive_exact_result() {
    for sse in [false, true] {
        exercise_http(70_000, 0, 1, sse);
    }
}

#[test]
fn http_two_calls_each_with_33_notifications_complete() {
    exercise_http(0, 0, 2, true);
}

#[test]
fn http_tool_result_capacity_does_not_widen_notification_node_budget() {
    exercise_http(0, 70_000, 1, true);
}
