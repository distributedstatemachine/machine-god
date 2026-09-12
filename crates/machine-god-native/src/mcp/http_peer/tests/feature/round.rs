use super::*;
use crate::mcp::{
    control::{McpFeatureControlAuthority, McpFeatureRound},
    mrtr::McpValidatedResponses,
};
use serde_json::value::RawValue;

const INPUT: &[u8] = br#"{"jsonrpc":"2.0","id":2,"result":{"resultType":"input_required","inputRequests":{"ask":{"method":"elicitation/create","params":{"message":"Authorize","mode":"url","url":"https://example.test/auth"}}},"requestState":{"number":9007199254740993,"small":1e-99999}}}"#;

fn answers(round: &McpFeatureRound) -> McpValidatedResponses {
    round
        .input()
        .unwrap()
        .validate_responses(
            &RawValue::from_string(r#"{"ask":{"action":"accept"}}"#.into()).unwrap(),
        )
        .unwrap()
}

async fn first(peer: &mut McpHttpPeer, authority: McpFeatureControlAuthority) -> McpFeatureRound {
    peer.feature_round(
        &feature_request("resource read srv test://fixed"),
        "srv",
        &catalogs(),
        authority,
        McpFeatureOperationOptions::new(Instant::now()),
        deadline(),
    )
    .await
    .unwrap()
}

#[test]
fn modern_read_get_rounds_retain_original_params_metadata_and_exact_state() {
    for (command, result) in [
        ("resource read srv test://fixed", r#""contents":[]"#),
        (
            r#"prompt get srv review {"topic":"original"}"#,
            r#""messages":[]"#,
        ),
    ] {
        executor().block_on(async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let selected = options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            );
            let server = async {
                accept_reply(&listener, 200, JSON, &discovery()).await;
                let original = accept_reply(&listener, 200, JSON, INPUT).await;
                let result = format!(
                    r#"{{"jsonrpc":"2.0","id":3,"result":{{"resultType":"complete",{result}}}}}"#
                );
                let resumed = accept_reply(&listener, 200, JSON, result.as_bytes()).await;
                let body = |wire: &[u8]| {
                    let offset = memchr::memmem::find(wire, b"\r\n\r\n").unwrap() + 4;
                    machine_god_core::json::from_slice(&wire[offset..]).unwrap()
                };
                let before = body(&original);
                let after = body(&resumed);
                assert_eq!(before["id"], 2);
                assert_eq!(after["id"], 3);
                assert_eq!(before["method"], after["method"]);
                for key in ["uri", "name", "arguments", "_meta"] {
                    assert_eq!(before["params"][key], after["params"][key]);
                }
                let resumed = String::from_utf8(resumed).unwrap();
                assert!(resumed.contains("9007199254740993"));
                assert!(resumed.contains("1e-99999"));
                assert_eq!(after["params"]["inputResponses"]["ask"]["action"], "accept");
            };
            let client = async {
                let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                    .await
                    .unwrap();
                let mut settings = McpFeatureOperationOptions::new(Instant::now());
                settings.form = true;
                settings.url = true;
                settings.progress_token = Some(918);
                let round = peer
                    .feature_round(
                        &feature_request(command),
                        "srv",
                        &catalogs(),
                        human(CancellationToken::new()),
                        settings,
                        deadline(),
                    )
                    .await
                    .unwrap();
                let input = round.input().unwrap().clone();
                let responses = answers(&round);
                let complete = peer
                    .resume_feature(round, responses, deadline())
                    .await
                    .unwrap();
                assert!(complete.input().is_none());
                assert!(matches!(
                    complete.into_reply(),
                    McpFeatureReply::Response(_)
                ));
                assert_eq!(peer.next_id, Some(4));
                assert_eq!(input.requests()[0].key(), "ask");
                assert_eq!(peer.response_limits, WireLimits::default());
                assert!(peer.feature_authority.is_none());
            };
            tokio::time::timeout(Duration::from_secs(3), join(server, require_send(client)))
                .await
                .unwrap();
        });
    }
}

#[test]
fn unpolled_resume_consumes_no_id_and_stale_authority_cannot_write() {
    for cancelled in [false, true] {
        executor().block_on(async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let selected = options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            );
            let server = async {
                accept_reply(&listener, 200, JSON, &discovery()).await;
                accept_reply(&listener, 200, JSON, INPUT).await;
            };
            let client = async {
                let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                    .await
                    .unwrap();
                let cancellation = CancellationToken::new();
                let round = first(&mut peer, human(cancellation.clone())).await;
                let responses = answers(&round);
                if cancelled {
                    cancellation.cancel();
                    assert!(
                        peer.resume_feature(round, responses, deadline())
                            .await
                            .is_err()
                    );
                } else {
                    drop(peer.resume_feature(round, responses, deadline()));
                }
                assert_eq!(peer.next_id, Some(3));
                assert!(!peer.closed);
            };
            tokio::time::timeout(Duration::from_secs(3), join(server, client))
                .await
                .unwrap();
            assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
        });
    }
}

#[test]
fn continuation_rejects_a_distinct_peer_with_equal_endpoint_and_request_sequence() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = || {
            options(
                listener.local_addr().unwrap(),
                TransportKind::StreamableHttp,
            )
        };
        let server = async {
            accept_reply(&listener, 200, JSON, &discovery()).await;
            accept_reply(&listener, 200, JSON, INPUT).await;
            accept_reply(&listener, 200, JSON, &discovery()).await;
            accept_reply(&listener, 200, JSON, INPUT).await;
        };
        let client = async {
            let mut original =
                McpHttpPeer::connect(selected(), CancellationToken::new(), deadline())
                    .await
                    .unwrap();
            let round = first(&mut original, human(CancellationToken::new())).await;
            let responses = answers(&round);
            let mut foreign =
                McpHttpPeer::connect(selected(), CancellationToken::new(), deadline())
                    .await
                    .unwrap();
            drop(first(&mut foreign, human(CancellationToken::new())).await);
            assert_eq!(foreign.next_id, original.next_id);
            assert!(
                foreign
                    .resume_feature(round, responses, deadline())
                    .await
                    .is_err()
            );
            assert_eq!(foreign.next_id, Some(3));
            assert!(!foreign.closed);
            assert!(!original.closed);
        };
        tokio::time::timeout(Duration::from_secs(3), join(server, client))
            .await
            .unwrap();
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
    });
}

#[test]
fn dropped_partial_resume_keeps_original_guard_and_owned_cleanup() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let partial = CancellationToken::new();
        let server = async {
            accept_reply(&listener, 200, JSON, &discovery()).await;
            accept_reply(&listener, 200, JSON, INPUT).await;
            let mut socket = listener.accept().await.unwrap().0;
            let written = request(&mut socket).await;
            assert!(String::from_utf8_lossy(&written).contains("inputResponses"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: {")
                .await
                .unwrap();
            socket.flush().await.unwrap();
            partial.cancel();
            match socket.read(&mut [0]).await {
                Ok(0) => {}
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                result => panic!("expected owned connection closure, got {result:?}"),
            }
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let round = first(&mut peer, human(CancellationToken::new())).await;
            let responses = answers(&round);
            {
                let result = futures_util::future::select(
                    Box::pin(peer.resume_feature(round, responses, deadline())),
                    partial.cancelled(),
                )
                .await;
                let futures_util::future::Either::Right(((), pending)) = result else {
                    panic!("resume finished before correlated reply")
                };
                drop(pending);
            }
            assert!(peer.closed);
            peer.completion().completed().await;
        };
        tokio::time::timeout(Duration::from_secs(3), join(server, client))
            .await
            .unwrap();
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
    });
}
