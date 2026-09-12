use super::*;
use crate::mcp::submission::tests::Fixture;

#[test]
fn leased_requests_release_only_their_own_slots_and_reject_manual_interference() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let server = async {
            accept_reply(&listener, 200, JSON, &modern(1)).await;
            let request = accept_reply(
                &listener,
                200,
                JSON,
                &success(6, serde_json::json!({"content":[]})),
            )
            .await;
            let split = memchr::memmem::find(&request, b"\r\n\r\n").unwrap() + 4;
            let body = machine_god_core::json::from_slice(&request[split..]).unwrap();
            assert_eq!(body["id"], 6);
            assert_eq!(body["method"], "tools/call");
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let fixture = Fixture::new();
            peer.admit_runtimes(vec![fixture.runtime.clone()]).unwrap();
            let abandoned = peer.reserve_tool().unwrap();
            assert_eq!(abandoned.rpc_id(), &RpcId::Integer(2));
            assert!(peer.reserve_tool_id().is_err());
            drop(abandoned);
            let head = Arc::new(peer.make_head(Some("tools/call")).unwrap());
            let unsubmitted = peer.reserve_tool().unwrap();
            assert_eq!(unsubmitted.rpc_id(), &RpcId::Integer(3));
            fixture.ready_http_with_reservation("unsubmitted", &head, unsubmitted);
            assert!(peer.reserve_tool_id().is_err());
            let claimed = fixture
                .claim("unsubmitted", CancellationToken::new())
                .await
                .unwrap();
            assert!(peer.reserve_tool_id().is_err());
            drop(claimed);
            let stale = peer.reserve_tool().unwrap();
            assert_eq!(stale.rpc_id(), &RpcId::Integer(4));
            peer.discard_tool_id();
            assert!(peer.reserve_tool_id().is_err());
            drop(stale);
            assert_eq!(peer.reserve_tool_id().unwrap(), RpcId::Integer(5));
            assert!(peer.reserve_tool().is_err());
            peer.discard_tool_id();
            let actual = peer.reserve_tool().unwrap();
            assert_eq!(actual.rpc_id(), &RpcId::Integer(6));
            fixture.ready_http_with_id("no-marker", &head, actual.rpc_id().clone());
            let forged = fixture
                .claim("no-marker", CancellationToken::new())
                .await
                .unwrap();
            assert!(matches!(
                peer.call(forged, head.clone(), deadline()).await,
                Err(McpHttpPeerError::Correlation)
            ));
            assert!(peer.reserve_tool_id().is_err());
            fixture.ready_http_with_reservation("actual", &head, actual);
            let submission = fixture
                .claim("actual", CancellationToken::new())
                .await
                .unwrap();
            let response = peer.call(submission, head, deadline()).await.unwrap();
            assert_eq!(response.envelope().id(), Some(&RpcId::Integer(6)));
            assert!(peer.reserve_tool().is_ok());
        };
        join(client, server).await;
    });
}

#[test]
fn independently_prepared_leases_can_be_submitted_out_of_order() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let server = async {
            accept_reply(&listener, 200, JSON, &modern(1)).await;
            for id in [3, 2] {
                let request = accept_reply(
                    &listener,
                    200,
                    JSON,
                    &success(id, serde_json::json!({"content":[]})),
                )
                .await;
                let split = memchr::memmem::find(&request, b"\r\n\r\n").unwrap() + 4;
                let body = machine_god_core::json::from_slice(&request[split..]).unwrap();
                assert_eq!(body["id"], id);
                assert_eq!(body["method"], "tools/call");
            }
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let fixture = Fixture::new();
            peer.admit_runtimes(vec![fixture.runtime.clone()]).unwrap();
            let head = Arc::new(peer.make_head(Some("tools/call")).unwrap());
            let first = peer.reserve_tool().unwrap();
            let second = peer.reserve_tool().unwrap();
            assert_eq!(first.rpc_id(), &RpcId::Integer(2));
            assert_eq!(second.rpc_id(), &RpcId::Integer(3));
            fixture.ready_http_with_reservation("first", &head, first);
            fixture.ready_http_with_reservation("second", &head, second);
            peer.discard_tool_id();
            assert!(peer.reserve_tool_id().is_err());
            for (name, id) in [("second", 3), ("first", 2)] {
                let submission = fixture.claim(name, CancellationToken::new()).await.unwrap();
                let response = require_send(peer.call(submission, head.clone(), deadline()))
                    .await
                    .unwrap();
                assert_eq!(response.envelope().id(), Some(&RpcId::Integer(id)));
            }
            assert!(peer.reserve_tool_id().is_ok());
        };
        join(client, server).await;
    });
}

#[test]
fn allocated_tool_id_and_exact_native_proof_reach_owned_http_writer() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let server = async {
            accept_reply(&listener, 200, JSON, &modern(1)).await;
            let request = accept_reply(
                &listener,
                200,
                JSON,
                &success(2, serde_json::json!({"content":[]})),
            )
            .await;
            let split = memchr::memmem::find(&request, b"\r\n\r\n").unwrap() + 4;
            let body: serde_json::Value = serde_json::from_slice(&request[split..]).unwrap();
            assert_eq!(body["id"], 2);
            assert_eq!(body["method"], "tools/call");
            assert_eq!(body["params"]["name"], "secret-tool");
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let fixture = Fixture::new();
            peer.admit_runtimes(vec![fixture.runtime.clone()]).unwrap();
            let id = peer.reserve_tool_id().unwrap();
            let head = Arc::new(peer.make_head(Some("tools/call")).unwrap());
            fixture.ready_http_with_id("actual-call", &head, id.clone());
            let submission = fixture
                .claim("actual-call", CancellationToken::new())
                .await
                .unwrap();
            let response = peer.call(submission, head, deadline()).await.unwrap();
            assert_eq!(response.envelope().id(), Some(&id));
            assert!(peer.reserve_tool_id().is_ok());
        };
        join(client, server).await;
    });
}

#[test]
fn changed_selected_headers_or_runtime_allocation_fail_before_socket_acquisition() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let discovery = modern(1);
        let server = accept_reply(&listener, 200, JSON, &discovery);
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let first = Fixture::new();
            peer.admit_runtimes(vec![first.runtime.clone()]).unwrap();
            let id = peer.reserve_tool_id().unwrap();
            let head = Arc::new(peer.make_head(Some("tools/call")).unwrap());
            first.ready_http_with_id("changed-head", &head, id);
            let submission = first
                .claim("changed-head", CancellationToken::new())
                .await
                .unwrap();
            let changed = Arc::new(
                McpSubmissionHttpHead::new(
                    head.endpoint(),
                    &[
                        ("X-Selected", b"changed"),
                        ("mcp-protocol-version", b"2026-07-28"),
                    ],
                )
                .unwrap(),
            );
            assert!(matches!(
                peer.call(submission, changed, deadline()).await,
                Err(McpHttpPeerError::Invalid)
            ));
            peer.discard_tool_id();
            let id = peer.reserve_tool_id().unwrap();
            let second = Fixture::new();
            second.ready_http_with_id("foreign-runtime", &head, id);
            let submission = second
                .claim("foreign-runtime", CancellationToken::new())
                .await
                .unwrap();
            assert!(matches!(
                peer.call(submission, head, deadline()).await,
                Err(McpHttpPeerError::Invalid)
            ));
        };
        join(client, server).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
    });
}

#[test]
fn tool_cancellation_during_partial_sse_releases_socket_without_replay() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let cancellation = CancellationToken::new();
        let server = async {
            accept_reply(&listener, 200, JSON, &modern(1)).await;
            let (mut socket, _) = listener.accept().await.unwrap();
            let sent = request(&mut socket).await;
            assert!(String::from_utf8_lossy(&sent).contains("tools/call"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: {")
                .await
                .unwrap();
            socket.flush().await.unwrap();
            cancellation.cancel();
            assert_eq!(socket.read(&mut [0]).await.unwrap(), 0);
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let fixture = Fixture::new();
            peer.admit_runtimes(vec![fixture.runtime.clone()]).unwrap();
            let id = peer.reserve_tool_id().unwrap();
            let head = Arc::new(peer.make_head(Some("tools/call")).unwrap());
            fixture.ready_http_with_id("cancelled-call", &head, id);
            let submission = fixture
                .claim("cancelled-call", cancellation.clone())
                .await
                .unwrap();
            assert!(peer.call(submission, head, deadline()).await.is_err());
            peer.completion().completed().await;
            assert!(peer.reserve_tool_id().is_err());
        };
        tokio::time::timeout(Duration::from_secs(2), join(client, server))
            .await
            .unwrap();
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
    });
}
