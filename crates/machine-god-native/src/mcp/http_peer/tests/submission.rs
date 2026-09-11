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
            legacy_start(&listener, "2025-11-25", "leased-session").await;
            let request = accept_reply(
                &listener,
                200,
                JSON,
                &success(7, serde_json::json!({"content":[]})),
            )
            .await;
            let split = memchr::memmem::find(&request, b"\r\n\r\n").unwrap() + 4;
            let body = machine_god_core::json::from_slice(&request[split..]).unwrap();
            assert_eq!(body["id"], 7);
            assert_eq!(body["method"], "tools/call");
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let fixture = Fixture::new();
            peer.admit_runtimes(vec![fixture.runtime.clone()]).unwrap();
            let abandoned = peer.reserve_tool().unwrap();
            assert_eq!(abandoned.rpc_id(), &RpcId::Integer(3));
            assert!(peer.reserve_tool_id().is_err());
            drop(abandoned);
            let head = Arc::new(peer.request_head().unwrap());
            let unsubmitted = peer.reserve_tool().unwrap();
            assert_eq!(unsubmitted.rpc_id(), &RpcId::Integer(4));
            fixture.ready_http_with_reservation("unsubmitted", &head, unsubmitted);
            assert!(peer.reserve_tool_id().is_err());
            let claimed = fixture
                .claim("unsubmitted", CancellationToken::new())
                .await
                .unwrap();
            assert!(peer.reserve_tool_id().is_err());
            drop(claimed);
            let stale = peer.reserve_tool().unwrap();
            assert_eq!(stale.rpc_id(), &RpcId::Integer(5));
            peer.discard_tool_id();
            assert!(peer.reserve_tool_id().is_err());
            drop(stale);
            assert_eq!(peer.reserve_tool_id().unwrap(), RpcId::Integer(6));
            assert!(peer.reserve_tool().is_err());
            peer.discard_tool_id();
            let actual = peer.reserve_tool().unwrap();
            assert_eq!(actual.rpc_id(), &RpcId::Integer(7));
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
            assert_eq!(response.envelope().id(), Some(&RpcId::Integer(7)));
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
            legacy_start(&listener, "2025-11-25", "parallel-preparation").await;
            for id in [4, 3] {
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
            let head = Arc::new(peer.request_head().unwrap());
            let first = peer.reserve_tool().unwrap();
            let second = peer.reserve_tool().unwrap();
            assert_eq!(first.rpc_id(), &RpcId::Integer(3));
            assert_eq!(second.rpc_id(), &RpcId::Integer(4));
            fixture.ready_http_with_reservation("first", &head, first);
            fixture.ready_http_with_reservation("second", &head, second);
            peer.discard_tool_id();
            assert!(peer.reserve_tool_id().is_err());
            for (name, id) in [("second", 4), ("first", 3)] {
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
            legacy_start(&listener, "2025-11-25", "tool-session").await;
            let request = accept_reply(
                &listener,
                200,
                JSON,
                &success(3, serde_json::json!({"content":[]})),
            )
            .await;
            let split = memchr::memmem::find(&request, b"\r\n\r\n").unwrap() + 4;
            let body: serde_json::Value = serde_json::from_slice(&request[split..]).unwrap();
            assert_eq!(body["id"], 3);
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
            let head = Arc::new(peer.request_head().unwrap());
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
        let server = legacy_start(&listener, "2025-11-25", "strict");
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let first = Fixture::new();
            peer.admit_runtimes(vec![first.runtime.clone()]).unwrap();
            let id = peer.reserve_tool_id().unwrap();
            let head = Arc::new(peer.request_head().unwrap());
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
                        ("mcp-protocol-version", b"2025-11-25"),
                        ("mcp-session-id", b"strict"),
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
fn deprecated_sse_tool_cancellation_keeps_scope_observed_after_post_ack() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(listener.local_addr().unwrap(), TransportKind::LegacySse);
        let cancellation = CancellationToken::new();
        let server = async {
            let (mut events, _) = listener.accept().await.unwrap();
            request(&mut events).await;
            events.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\nevent: endpoint\ndata: /messages\n\n").await.unwrap();
            let (mut init, _) = listener.accept().await.unwrap();
            request(&mut init).await;
            events.write_all(b"data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{}}}\n\n").await.unwrap();
            init.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n").await.unwrap();
            accept_reply(&listener, 202, "", b"").await;
            let tool = accept_reply(&listener, 202, "", b"").await;
            assert!(String::from_utf8_lossy(&tool).contains("tools/call"));
            cancellation.cancel();
            let cancelled = accept_reply(&listener, 202, "", b"").await;
            assert!(String::from_utf8_lossy(&cancelled).contains("notifications/cancelled"));
            assert!(!String::from_utf8_lossy(&cancelled).contains("tools/call"));
            let mut byte = [0];
            assert_eq!(events.read(&mut byte).await.unwrap(), 0);
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline()).await.unwrap();
            let fixture = Fixture::new();
            peer.admit_runtimes(vec![fixture.runtime.clone()]).unwrap();
            let id = peer.reserve_tool_id().unwrap();
            let head = Arc::new(peer.request_head().unwrap());
            fixture.ready_http_with_id("cancelled-call", &head, id);
            let submission = fixture.claim("cancelled-call", cancellation.clone()).await.unwrap();
            assert!(peer.call(submission, head, deadline()).await.is_err());
            peer.completion().completed().await;
        };
        join(client, server).await;
    });
}
