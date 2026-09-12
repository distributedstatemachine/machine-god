//! Real localhost HTTP peer and host; no configured-startup or OAuth claim.
use super::support;
use machine_god_core::CancellationToken;
use machine_god_native as native;
use native::mcp::{
    catalog::{McpDescriptorCatalog, McpDescriptorLimits},
    clock::TokioMcpClock,
    config::{McpServerConfig, McpTransportConfig},
    context::NativeMcpContexts,
    endpoint::McpEndpoint,
    headers::McpResolvedHeaders,
    http::McpHttpDestination,
    http_peer::{McpHttpPeer, McpHttpPeerOptions},
    lifetime::McpPeerLifetime,
    pagination::{McpCatalogKind, McpCatalogLimits},
    protocol::TransportKind,
    runtime::{NativeMcpOwnedPeer, NativeMcpServerCandidate},
};
use native::{
    NativeInteractivePromptBridge, NativeInteractivePromptInbox, NativeInteractivePromptLimits,
};
use serde_json::{Value, json};
use std::net::Ipv4Addr;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub(super) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

pub(super) struct HttpFixture {
    pub fixture: support::Fixture,
    pub bridge: Arc<NativeInteractivePromptBridge>,
    pub inbox: NativeInteractivePromptInbox,
    pub listener: TcpListener,
}

pub(super) async fn setup() -> HttpFixture {
    let (bridge, inbox) =
        NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
    let clock = Arc::new(TokioMcpClock);
    let options = native::NativeReferenceHostMcpOptions::new(
        Arc::new(NativeMcpContexts::new()),
        clock.clone(),
    )
    .with_form_responder(bridge.clone());
    let fixture = support::Fixture::with_host_options(|host| host.with_mcp_runtime(options), None);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}/mcp");
    let config = McpServerConfig::decode(
        "fixture",
        &serde_json::to_vec(&json!({"type":"http","url":url})).unwrap(),
    )
    .unwrap();
    let McpTransportConfig::Http(remote) = config.transport() else {
        unreachable!()
    };
    let client = async {
        let mut peer = McpHttpPeer::connect(
            McpHttpPeerOptions {
                destination: McpHttpDestination::new(McpEndpoint::parse(&url).unwrap(), &[address])
                    .unwrap(),
                trust: None,
                headers: McpResolvedHeaders::resolve(remote, |_| None, None, &[]).unwrap(),
                clock,
                transport: TransportKind::StreamableHttp,
                lifetime: McpPeerLifetime::OwnerControlled,
            },
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
        let runtime = fixture.host.mcp_runtime().unwrap();
        let reserved = fixture
            .host
            .reserved_tool_names()
            .iter()
            .map(machine_god_core::ToolName::as_str)
            .collect::<Vec<_>>();
        let candidate = runtime
            .prepare_candidate(
                vec![NativeMcpServerCandidate {
                    server: Arc::from("fixture"),
                    configuration: Arc::from(&b"explicit-local-fixture"[..]),
                    authentication: Arc::from([]),
                    catalogs: vec![catalog],
                    refresh: None,
                    peer: NativeMcpOwnedPeer::Http(Box::new(peer)),
                    operation_timeout: Duration::from_secs(5),
                    authority_cancellations: Arc::from([]),
                    catalog_epoch: epoch,
                }],
                &reserved,
            )
            .unwrap();
        runtime.publish(candidate).unwrap();
    };
    let server = async {
        reply(&listener, "server/discover", json!({"supportedVersions":["2026-07-28"],"capabilities":{"tools":{},"resources":{},"prompts":{},"completions":{}}})).await;
        reply(&listener, "tools/list", json!({"tools":[],"ttlMs":300_000})).await;
    };
    futures_util::future::join(client, server).await;
    HttpFixture {
        fixture,
        bridge,
        inbox,
        listener,
    }
}

pub(super) async fn reply(listener: &TcpListener, method: &str, mut result: Value) -> Value {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut bytes = Vec::new();
    let request = loop {
        let mut scratch = [0; 4096];
        let count = socket.read(&mut scratch).await.unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&scratch[..count]);
        assert!(bytes.len() <= 256 * 1024);
        let Some(head) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
            continue;
        };
        let headers = std::str::from_utf8(&bytes[..head]).unwrap();
        assert!(headers.starts_with("POST /mcp HTTP/1.1\r\n"));
        let length: usize = headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
            .unwrap()
            .1
            .trim()
            .parse()
            .unwrap();
        if bytes.len() < head + 4 + length {
            continue;
        }
        assert_eq!(bytes.len(), head + 4 + length);
        break machine_god_core::json::from_slice(&bytes[head + 4..]).unwrap();
    };
    assert_eq!(request["method"], method);
    assert_eq!(
        request["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
        "2026-07-28"
    );
    assert!(request["id"].as_i64().is_some_and(|id| id > 0));
    if result.get("resultType").is_none() {
        result["resultType"] = "complete".into();
    }
    let body =
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":request["id"],"result":result})).unwrap();
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
    socket.write_all(&body).await.unwrap();
    socket.shutdown().await.unwrap();
    request
}

pub(super) fn resources() -> Value {
    json!({"resources":[{"uri":"test://fixed","name":"fixed"}],"ttlMs":300_000})
}
pub(super) fn prompts() -> Value {
    json!({"prompts":[{"name":"review","arguments":[{"name":"topic","required":true}]}],"ttlMs":300_000})
}
pub(super) fn templates() -> Value {
    json!({"resourceTemplates":[{"uriTemplate":"test:///{id}","name":"dynamic"}],"ttlMs":300_000})
}
