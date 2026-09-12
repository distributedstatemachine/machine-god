use super::*;
use crate::mcp::{
    catalog::{McpDescriptorCatalog, McpDescriptorLimits},
    config::{McpServerConfig, McpTransportConfig},
    endpoint::McpEndpoint,
    headers::McpResolvedHeaders,
    http::{McpHttpClock, McpHttpDestination, tests::request},
    http_peer::{McpHttpPeer, McpHttpPeerOptions},
    pagination::{McpCatalogKind, McpCatalogLimits},
    protocol::TransportKind,
    runtime::{NativeMcpOwnedPeer, NativeMcpServerCandidate},
};
use futures_util::future::join;
use machine_god_core::{ContentBlock, SessionRecord, ToolOutput};
use serde_json::json;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::{io::AsyncWriteExt, net::TcpListener};

impl McpHttpClock for Clock {
    fn now(&self) -> Instant {
        NativeMcpRuntimeClock::now(self)
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        NativeMcpRuntimeClock::sleep_until(self, deadline)
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn options(address: SocketAddr, clock: Arc<Clock>) -> McpHttpPeerOptions {
    let url = format!("http://127.0.0.1:{}/mcp", address.port());
    let config = McpServerConfig::decode(
        "fixture",
        &serde_json::to_vec(&json!({"type":"http","url":url})).unwrap(),
    )
    .unwrap();
    let McpTransportConfig::Http(remote) = config.transport() else {
        panic!()
    };
    McpHttpPeerOptions {
        destination: McpHttpDestination::new(McpEndpoint::parse(&url).unwrap(), &[address])
            .unwrap(),
        trust: None,
        headers: McpResolvedHeaders::resolve(remote, |_| None, None, &[]).unwrap(),
        clock,
        transport: TransportKind::StreamableHttp,
        lifetime_deadline: deadline(),
    }
}
async fn reply(listener: &TcpListener, body: &[u8]) -> Vec<u8> {
    let (mut socket, _) = listener.accept().await.unwrap();
    let received = request(&mut socket).await;
    socket
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    socket.write_all(body).await.unwrap();
    socket.flush().await.unwrap();
    received
}
async fn publish(fixture: &Fixture, listener: &TcpListener) -> String {
    let client = async {
        let mut peer = McpHttpPeer::connect(
            options(listener.local_addr().unwrap(), fixture.clock.clone()),
            CancellationToken::new(),
            deadline(),
        )
        .await
        .unwrap();
        let epoch = Instant::now();
        let raw = peer
            .catalog(
                McpCatalogKind::Tools,
                McpCatalogLimits::default(),
                epoch,
                deadline(),
            )
            .await
            .unwrap();
        let catalog = McpDescriptorCatalog::admit(raw, McpDescriptorLimits::default()).unwrap();
        let runtime = fixture.host().mcp_runtime().unwrap();
        let reserved = fixture
            .host()
            .reserved_tool_names()
            .iter()
            .map(ToolName::as_str)
            .collect::<Vec<_>>();
        let candidate = runtime
            .prepare_candidate(
                vec![NativeMcpServerCandidate {
                    server: Arc::from("fixture"),
                    configuration: Arc::from(&b"configuration"[..]),
                    authentication: Arc::from(&b"authentication"[..]),
                    catalogs: vec![catalog],
                    peer: NativeMcpOwnedPeer::Http(Box::new(peer)),
                    operation_timeout: Duration::from_secs(5),
                    authority_cancellations: Arc::from([]),
                    catalog_epoch: epoch,
                }],
                &reserved,
            )
            .unwrap();
        let name = candidate.descriptors().tools()[0].name().to_owned();
        runtime.publish(candidate).unwrap();
        name
    };
    let server = async {
        reply(listener, br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}}}}"#).await;
        reply(listener, br#"{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","tools":[{"name":"lookup","inputSchema":{"type":"object"}}]}}"#).await;
    };
    join(client, server).await.0
}
fn output(events: &[TurnEvent], id: &str) -> ToolOutput {
    events
        .iter()
        .find_map(|event| match event {
            TurnEvent::ToolFinished { call_id, output } if call_id.as_str() == id => {
                Some(output.clone())
            }
            _ => None,
        })
        .unwrap()
}
fn persisted(record: &SessionRecord, id: &str) -> ToolOutput {
    record
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|block| match block {
            ContentBlock::ToolResult { call_id, output } if call_id.as_str() == id => {
                Some(output.clone())
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn actual_http_tool_auto_review_and_read_tool_result_share_native_archive() {
    let fixture = Fixture::new("auto", true);
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let name = publish(&fixture, &listener).await;
        let arguments = machine_god_core::json::from_str(r#"{"n":9007199254740993.0000001,"tiny":1e-99999,"$serde_json::private::Number":"literal"}"#).unwrap();
        fixture.transport.responses.lock().unwrap().extend([
            call("select", MCP_SELECT_TOOL_NAME, &json!({"name":name})),
            call("execute", &name, &arguments),
            answer(),
        ]);
        let conversation = fixture.conversation().await;
        let runtime = NativeConversationRuntime::new(
            conversation,
            fixture.host().loaded_config().config().model_preferences(),
            None,
        )
        .unwrap();
        let raw = format!(
            r#"{{"resultType":"complete","structuredContent":{{"n":9007199254740993.0000001,"tiny":1e-99999,"$serde_json::private::Number":"literal"}},"content":[{{"type":"text","text":"{}"}}]}}"#,
            "x".repeat(70_000)
        );
        let response = format!(r#"{{"jsonrpc":"2.0","id":3,"result":{raw}}}"#);
        let (events, sent) = join(collect(&runtime), reply(&listener, response.as_bytes())).await;
        assert!(!output(&events, "execute").is_error);
        let body = sent
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap()
            + 4;
        let sent = machine_god_core::json::from_slice(&sent[body..]).unwrap();
        assert_eq!(sent["params"]["arguments"], arguments);
        assert_eq!(fixture.transport.reviews.load(Ordering::Relaxed), 1);
        assert_eq!(fixture.prompt.calls.load(Ordering::Relaxed), 0);
        let durable = persisted(&runtime.record(), "execute");
        assert_eq!(durable.content["type"], "tool_result_archive");
        fixture.transport.responses.lock().unwrap().extend([
            call("page", READ_TOOL_RESULT_TOOL_NAME, &json!({"handle":durable.content["archive"]["handle"],"start_byte":69_000,"byte_count":16384})), answer(),
        ]);
        let events = collect(&runtime).await;
        let page = output(&events, "page");
        assert!(!page.is_error, "{page:?}");
        let serialized = serde_json::to_string(&page.content).unwrap();
        assert!(serialized.contains("9007199254740993.0000001"));
        assert!(serialized.contains("$serde_json::private::Number"));
        fixture.host().close_mcp();
        let receipts = fixture
            .host()
            .drain_mcp(deadline(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(receipts.len(), 1);
        assert!(receipts[0].is_complete());
    });
}

#[test]
fn actual_http_tool_denial_writes_no_application_request() {
    let fixture = Fixture::new("ask", true);
    let archive_entries = fs::read_dir(fixture.state.join("tool-result-archive"))
        .unwrap()
        .count();
    fixture.prompt.deny.store(true, Ordering::Relaxed);
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let name = publish(&fixture, &listener).await;
        fixture.transport.responses.lock().unwrap().extend([
            call("select", MCP_SELECT_TOOL_NAME, &json!({"name":name})),
            call("execute", &name, &json!({})),
            answer(),
        ]);
        let conversation = fixture.conversation().await;
        let runtime = NativeConversationRuntime::new(
            conversation,
            fixture.host().loaded_config().config().model_preferences(),
            None,
        )
        .unwrap();
        let events = collect(&runtime).await;
        assert!(events.iter().any(|event| matches!(
            event,
            TurnEvent::PermissionResolved {
                decision: machine_god_core::PermissionDecision::Deny { .. },
                ..
            }
        )));
        assert_eq!(
            persisted(&runtime.record(), "execute").content["code"],
            "permission_denied"
        );
        assert_eq!(fixture.prompt.calls.load(Ordering::Relaxed), 1);
        assert_eq!(fixture.transport.reviews.load(Ordering::Relaxed), 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
        assert_eq!(
            fs::read_dir(fixture.state.join("tool-result-archive"))
                .unwrap()
                .count(),
            archive_entries
        );
        fixture.host().close_mcp();
        assert!(
            fixture
                .host()
                .drain_mcp(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .iter()
                .all(crate::mcp::runtime::NativeMcpPeerCompletion::is_complete)
        );
    });
}
