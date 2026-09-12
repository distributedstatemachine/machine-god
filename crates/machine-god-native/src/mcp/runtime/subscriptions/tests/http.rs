use super::*;
use crate::mcp::{
    config::{McpServerConfig, McpTransportConfig},
    control::McpFeatureControlAuthority,
    endpoint::McpEndpoint,
    headers::McpResolvedHeaders,
    http::{
        McpHttpClock, McpHttpDestination,
        tests::{executor, request},
    },
    http_peer::{McpHttpPeer, McpHttpPeerOptions},
    lifetime::McpPeerLifetime,
    protocol::TransportKind,
    runtime::NativeMcpRuntimeClock,
};
use futures_util::{future::join, lock::Mutex};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{
    net::Ipv4Addr,
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

struct Clock;
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}
impl McpHttpClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        NativeMcpRuntimeClock::sleep_until(self, deadline)
    }
}

fn authority() -> McpFeatureControlAuthority {
    McpFeatureControlAuthority::for_human(
        CancellationToken::new(),
        CancellationToken::new(),
        CancellationToken::new(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        Arc::from([]),
    )
    .unwrap()
}

async fn route(listener: &TcpListener) -> ServerRoute {
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}/mcp");
    let config = McpServerConfig::decode(
        "fixture",
        &serde_json::to_vec(&json!({"type":"http","url":url})).unwrap(),
    )
    .unwrap();
    let McpTransportConfig::Http(remote) = config.transport() else {
        panic!()
    };
    let options = McpHttpPeerOptions {
        destination: McpHttpDestination::new(McpEndpoint::parse(&url).unwrap(), &[address])
            .unwrap(),
        trust: None,
        headers: McpResolvedHeaders::resolve(remote, |_| None, None, &[]).unwrap(),
        clock: Arc::new(Clock),
        transport: TransportKind::StreamableHttp,
        lifetime: McpPeerLifetime::OwnerControlled,
    };
    let discover = async {
        let (mut socket, _) = listener.accept().await.unwrap();
        let received = request(&mut socket).await;
        assert!(String::from_utf8_lossy(&received).contains("server/discover"));
        let body = json!({"jsonrpc":"2.0","id":1,"result":{
            "resultType":"complete","supportedVersions":["2026-07-28"],
            "capabilities":{"resources":{"listChanged":true,"subscribe":true}}
        }})
        .to_string();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    };
    let (peer, ()) = join(
        McpHttpPeer::connect(
            options,
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
        ),
        discover,
    )
    .await;
    let peer = peer.unwrap();
    let protocol = peer.protocol();
    let peer = NativeMcpOwnedPeer::Http(Box::new(peer));
    ServerRoute {
        name: Arc::from("fixture"),
        configuration: Arc::from(&b"config"[..]),
        authentication: Arc::from(&b"auth"[..]),
        catalogs: std::sync::Mutex::new(NativeMcpCatalogState::new(&[]).unwrap()),
        results: std::sync::Mutex::new(crate::mcp::runtime::result_cache::FeatureResultCache::new(
            Arc::new(crate::mcp::runtime::catalog_state::FeatureCacheBudget::new(
                16 * 1024 * 1024,
            )),
        )),
        catalog_epoch: Instant::now(),
        protocol,
        readiness: peer.readiness(),
        peer: Mutex::new(peer),
        cancellation: CancellationToken::new(),
        pending: AtomicUsize::new(0),
        max_pending: 4,
        clock: Arc::new(Clock),
        timeout: Duration::from_secs(5),
        authority_cancellations: Arc::from([]),
    }
}

fn filters(uris: &[&str]) -> Value {
    let mut value = json!({"resourcesListChanged":true});
    if !uris.is_empty() {
        value["resourceSubscriptions"] = json!(uris);
    }
    value
}

fn ack_bytes(id: i64, notifications: &Value) -> Vec<u8> {
    format!("data: {}\n\n", json!({"jsonrpc":"2.0","method":"notifications/subscriptions/acknowledged",
        "params":{"_meta":{"io.modelcontextprotocol/subscriptionId":id},"notifications":notifications}})).into_bytes()
}

async fn listen(listener: &TcpListener, id: i64, notifications: Value) -> TcpStream {
    let (mut socket, _) = listener.accept().await.unwrap();
    let bytes = request(&mut socket).await;
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("POST /mcp HTTP/1.1"));
    let request: Value = serde_json::from_str(text.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(request["method"], "subscriptions/listen");
    assert_eq!(request["id"], id);
    assert_eq!(request["params"]["notifications"], notifications);
    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    socket
}

#[test]
fn direct_demand_resumes_partial_ack_then_expands_exact_uri_union() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let server = route(&listener).await;
        assert!(listener.accept().now_or_never().is_none());
        let partial = CancellationToken::new();
        let resume = CancellationToken::new();
        let finished = CancellationToken::new();
        let wire = async {
            let mut socket = listen(&listener, 2, filters(&[])).await;
            let ack = ack_bytes(2, &filters(&[]));
            let split = ack.len() / 2;
            socket.write_all(&ack[..split]).await.unwrap();
            partial.cancel();
            resume.cancelled().await;
            socket.write_all(&ack[split..]).await.unwrap();
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
            let mut socket = listen(&listener, 3, filters(&["file:///a"])).await;
            {
                let selected = state(&server).unwrap();
                let (reads, prompts) = selected
                    .policy
                    .result_cache_invalidation(&selected.generation)
                    .unwrap();
                assert!(reads > 0 && prompts > 0);
            }
            socket
                .write_all(&ack_bytes(3, &filters(&["file:///a"])))
                .await
                .unwrap();
            assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
            let mut socket = listen(&listener, 4, filters(&["file:///a", "file:///b"])).await;
            socket
                .write_all(&ack_bytes(4, &filters(&["file:///a", "file:///b"])))
                .await
                .unwrap();
            finished.cancelled().await;
        };
        let client = async {
            let owner = authority();
            let mut lane = server.acquire_feature(&owner).await.unwrap();
            {
                let pending = Box::pin(ensure(&mut lane, &server));
                let signal = Box::pin(async {
                    partial.cancelled().await;
                    while state(&server).unwrap().subscription.is_none() {
                        tokio::task::yield_now().await;
                    }
                });
                let futures_util::future::Either::Right(((), pending)) =
                    futures_util::future::select(pending, signal).await
                else {
                    panic!("ACK incomplete")
                };
                drop(pending);
            }
            assert_eq!(
                state(&server).unwrap().subscription,
                Some(RpcId::Integer(2))
            );
            assert!(!state(&server).unwrap().acknowledged);
            resume.cancel();
            ensure(&mut lane, &server).await.unwrap();
            ensure_resource(&mut lane, &server, "file:///a")
                .await
                .unwrap();
            ensure_resource(&mut lane, &server, "file:///a")
                .await
                .unwrap();
            assert_eq!(lane.peer.active_subscription(), Some(RpcId::Integer(3)));
            ensure_resource(&mut lane, &server, "file:///b")
                .await
                .unwrap();
            let prior = lane.peer.active_subscription();
            assert!(matches!(
                ensure_resource(&mut lane, &server, &"x".repeat(64 * 1024)).await,
                Err(Error::Limit)
            ));
            assert_eq!(lane.peer.active_subscription(), prior);
            assert_eq!(state(&server).unwrap().uris.len(), 2);
            assert!(listener.accept().now_or_never().is_none());
            finished.cancel();
        };
        join(client, wire).await;
    });
}

#[test]
fn failed_ack_closes_listener_and_disables_automatic_restart() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let server = route(&listener).await;
        let wire = async {
            let mut socket = listen(&listener, 2, filters(&[])).await;
            socket
                .write_all(&ack_bytes(2, &json!({"promptsListChanged":true})))
                .await
                .unwrap();
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
        };
        let client = async {
            let owner = authority();
            let mut lane = server.acquire_feature(&owner).await.unwrap();
            assert!(ensure(&mut lane, &server).await.is_err());
            assert!(state(&server).unwrap().subscription_stopped);
            assert!(lane.peer.active_subscription().is_none());
            ensure(&mut lane, &server).await.unwrap();
            ensure_resource(&mut lane, &server, "file:///never-started")
                .await
                .unwrap();
            assert!(listener.accept().now_or_never().is_none());
        };
        join(client, wire).await;
    });
}

#[test]
fn exact_listener_completion_invalidates_before_later_demand_restarts() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let server = route(&listener).await;
        let finish = CancellationToken::new();
        let done = CancellationToken::new();
        let wire = async {
            let mut socket = listen(&listener, 2, filters(&[])).await;
            socket.write_all(&ack_bytes(2, &filters(&[]))).await.unwrap();
            finish.cancelled().await;
            socket.write_all(b"data: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"resultType\":\"complete\",\"_meta\":{\"io.modelcontextprotocol/subscriptionId\":2}}}\n\n").await.unwrap();
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
            let mut next = listen(&listener, 3, filters(&[])).await;
            next.write_all(&ack_bytes(3, &filters(&[]))).await.unwrap();
            done.cancelled().await;
        };
        let client = async {
            let owner = authority();
            let mut lane = server.acquire_feature(&owner).await.unwrap();
            ensure(&mut lane, &server).await.unwrap();
            finish.cancel();
            loop {
                drain(&mut lane, &server).await.unwrap();
                if state(&server).unwrap().subscription.is_none() { break; }
                tokio::task::yield_now().await;
            }
            {
                let selected = state(&server).unwrap();
                assert!(!selected.subscription_stopped);
                assert_eq!(selected.policy.result_cache_invalidation(&selected.generation).unwrap(), (1, 1));
            }
            assert!(listener.accept().now_or_never().is_none());
            ensure(&mut lane, &server).await.unwrap();
            assert_eq!(lane.peer.active_subscription(), Some(RpcId::Integer(3)));
            done.cancel();
        };
        join(client, wire).await;
    });
}

#[test]
fn uri_expansion_observes_ordinary_stream_cancellation_before_handoff() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let server = route(&listener).await;
        let wire = async {
            let mut subscription = listen(&listener, 2, filters(&[])).await;
            subscription.write_all(&ack_bytes(2, &filters(&[]))).await.unwrap();
            let (mut catalog, _) = listener.accept().await.unwrap();
            let received = request(&mut catalog).await;
            assert!(String::from_utf8_lossy(&received).contains("resources/list"));
            let body = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":2}}\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"resultType\":\"complete\",\"resources\":[]}}\n\n";
            catalog.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes()).await.unwrap();
            catalog.write_all(body).await.unwrap();
            let mut byte = [0];
            assert_eq!(subscription.read(&mut byte).await.unwrap(), 0);
        };
        let client = async {
            let owner = authority();
            let mut lane = server.acquire_feature(&owner).await.unwrap();
            ensure(&mut lane, &server).await.unwrap();
            let deadline = lane.deadline;
            let NativeMcpOwnedPeer::Http(peer) = &mut *lane.peer else { panic!() };
            peer.catalog(McpCatalogKind::Resources, crate::mcp::pagination::McpCatalogLimits::default(), server.catalog_epoch, deadline).await.unwrap();
            ensure_resource(&mut lane, &server, "file:///not-restarted").await.unwrap();
            assert!(state(&server).unwrap().subscription_stopped);
            assert!(state(&server).unwrap().uris.is_empty());
            assert!(lane.peer.active_subscription().is_none());
            assert!(listener.accept().now_or_never().is_none());
        };
        join(client, wire).await;
    });
}
