//! Copy-only codec metadata acquired through a real public HTTP negotiation.

use machine_god_core::CancellationToken;
use machine_god_native::{
    NativeOwnedWorkerScope,
    mcp::{
        clock::TokioMcpClock,
        config::McpConfig,
        network::{McpResolverConfig, NativeMcpNetwork},
        peer::McpPeerCapabilities,
        protocol::NegotiatedProtocol,
        runtime::NativeMcpOwnedPeer,
        startup::{NativeMcpStartup, NativeMcpStartupOptions, NativeMcpStartupPhase},
    },
};
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub(super) fn negotiated() -> (NegotiatedProtocol, McpPeerCapabilities) {
    static METADATA: OnceLock<(NegotiatedProtocol, McpPeerCapabilities)> = OnceLock::new();
    *METADATA.get_or_init(|| {
        let workers = NativeOwnedWorkerScope::new();
        let metadata = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(10), acquire(workers.clone()))
                    .await
                    .expect("HTTP capability fixture must settle")
            });
        workers.close();
        workers.completion().wait_on_worker().unwrap();
        metadata
    })
}

async fn acquire(workers: NativeOwnedWorkerScope) -> (NegotiatedProtocol, McpPeerCapabilities) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let configuration = serde_json::json!({"mcp":{"srv":{"type":"http",
        "url":format!("http://{}/mcp", listener.local_addr().unwrap()),"required":true}}});
    let clock = Arc::new(TokioMcpClock);
    let deadline = Instant::now() + Duration::from_secs(10);
    let startup = NativeMcpStartup::new(NativeMcpStartupOptions {
        configuration: Arc::new(McpConfig::decode(configuration.to_string().as_bytes()).unwrap()),
        captured_environment: vec![],
        stdio: None,
        workers,
        clock: clock.clone(),
        catalog_epoch: Instant::now(),
        owner_cancellation: CancellationToken::new(),
        configuration_cancellation: CancellationToken::new(),
        network: Some(Arc::new(
            NativeMcpNetwork::new(
                McpResolverConfig::literal_only(),
                [17; 32],
                None,
                clock,
                CancellationToken::new(),
                1,
            )
            .unwrap(),
        )),
        authentication: vec![],
        peer_lifetime: deadline.into(),
        max_retained_bytes: 1024 * 1024,
    })
    .unwrap();
    let (batch, ()) = futures_util::future::join(
        startup.build(
            NativeMcpStartupPhase::All,
            CancellationToken::new(),
            deadline,
        ),
        discover(&listener),
    )
    .await;
    assert!(batch.receipt().required_ready());
    assert_eq!(batch.servers().len(), 1);
    let NativeMcpOwnedPeer::Http(peer) = &batch.servers()[0].peer else {
        panic!("actual HTTP peer");
    };
    let metadata = (peer.protocol(), peer.capabilities());
    drop(batch);
    while !startup.cleanup_observations().is_empty() {
        tokio::task::yield_now().await;
    }
    metadata
}

async fn discover(listener: &TcpListener) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    loop {
        assert!(request.len() < 16 * 1024);
        let mut chunk = [0; 1024];
        let count = socket.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0);
        request.extend_from_slice(&chunk[..count]);
        if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = std::str::from_utf8(&request[..header_end]).unwrap();
            let length = header
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .unwrap()
                .1
                .trim()
                .parse::<usize>()
                .unwrap();
            assert!(length < 16 * 1024);
            if request.len() >= header_end + 4 + length {
                break;
            }
        }
    }
    assert!(String::from_utf8_lossy(&request).contains("server/discover"));
    let body = br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"resources":{},"prompts":{},"completions":{}}}}"#;
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
    socket.write_all(body).await.unwrap();
    socket.flush().await.unwrap();
}
