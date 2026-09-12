use super::*;
use crate::mcp::{
    control::{
        McpFeatureOperationOptions, McpFeatureReply,
        tests::{catalogs, human, request as feature_request},
    },
    feature::McpFeatureOutcome,
};

fn discovery() -> Vec<u8> {
    success(
        1,
        serde_json::json!({"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"resources":{},"prompts":{},"completions":{}}}),
    )
}
#[test]
fn all_seven_typed_features_use_exact_heads_ids_and_complete_catalog_pages() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(listener.local_addr().unwrap(), TransportKind::StreamableHttp);
        let cases = [
            ("resource list srv", "resources/list", r#""resources":[{"uri":"test://fixed","name":"fixed"}]"#),
            ("resource templates srv", "resources/templates/list", r#""resourceTemplates":[{"uriTemplate":"test:///{id}","name":"dynamic"}]"#),
            ("resource read srv test://fixed", "resources/read", r#""contents":[{"uri":"test://fixed","text":"hello"}],"unknown":123456789012345678901234567890"#),
            ("prompt list srv", "prompts/list", r#""prompts":[{"name":"review"}]"#),
            (r#"prompt get srv review {"topic":"x"}"#, "prompts/get", r#""messages":[{"role":"assistant","content":{"type":"text","text":"hello"}}]"#),
            ("prompt complete srv review topic x", "completion/complete", r#""completion":{"values":["exact"],"total":1,"hasMore":false}"#),
            ("resource complete srv test:///{id} id x", "completion/complete", r#""completion":{"values":[]}"#),
        ];
        let server = async {
            accept_reply(&listener, 200, JSON, &discovery()).await;
            for (index, (_, method, fields)) in cases.iter().enumerate() {
                let id = index + 2;
                let body = format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"resultType":"complete",{fields}}}}}"#);
                let raw = accept_reply(&listener, 200, JSON, body.as_bytes()).await;
                let raw = String::from_utf8(raw).unwrap();
                assert!(raw.contains(&format!("mcp-method: {method}\r\n")));
                assert!(!raw.contains("mcp-name:"));
                assert!(!raw.contains("mcp-param-"));
                assert!(raw.contains(&format!("\"id\":{id}")));
                assert!(raw.contains("x-selected: secret\r\n"));
            }
            let first = accept_reply(&listener, 200, JSON, br#"{"jsonrpc":"2.0","id":9,"result":{"resultType":"complete","resources":[],"nextCursor":"page-2"}}"#).await;
            assert!(!String::from_utf8_lossy(&first).contains("cursor"));
            let second = accept_reply(&listener, 200, JSON, br#"{"jsonrpc":"2.0","id":10,"result":{"resultType":"complete","resources":[{"uri":"test://second","name":"second"}]}}"#).await;
            assert!(String::from_utf8_lossy(&second).contains("\"cursor\":\"page-2\""));
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline()).await.unwrap();
            let catalogs = catalogs();
            for (index, (command, _, _)) in cases.iter().enumerate() {
                let reply = peer.feature(&feature_request(command), "srv", &catalogs, human(CancellationToken::new()), McpFeatureOperationOptions::new(Instant::now()), deadline()).await.unwrap();
                if index == 2 { let McpFeatureReply::Response(response) = reply else { panic!() };
                    assert!(response.raw_json().get().contains("123456789012345678901234567890"));
                    assert!(matches!(response.outcome(), McpFeatureOutcome::Resource { .. }));
                }
                assert_eq!(peer.response_limits, WireLimits::default());
                assert!(peer.feature_authority.is_none());
            }
            let result = peer.feature(&feature_request("resource list srv"), "srv", &catalogs, human(CancellationToken::new()), McpFeatureOperationOptions::new(Instant::now()), deadline()).await.unwrap();
            assert!(matches!(result, McpFeatureReply::Catalog(_)));
            assert_eq!(peer.next_id, Some(11));
            peer.close();
            peer.completion().completed().await;
        };
        join(server, require_send(client)).await;
    });
}

#[test]
fn feature_authority_cancellation_stops_waiting_read_and_settles_owned_connection() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let cancelled = CancellationToken::new();
        let server = async {
            accept_reply(&listener, 200, JSON, &discovery()).await;
            let mut socket = listener.accept().await.unwrap().0;
            let raw = request(&mut socket).await;
            assert!(String::from_utf8_lossy(&raw).contains("resources/read"));
            cancelled.cancel();
            assert_eq!(socket.read(&mut [0; 1]).await.unwrap(), 0);
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let authority = crate::mcp::control::McpFeatureControlAuthority::for_human(
                CancellationToken::new(),
                CancellationToken::new(),
                CancellationToken::new(),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                Arc::from([cancelled.clone()]),
            )
            .unwrap();
            let result = peer
                .feature(
                    &feature_request("resource read srv test://fixed"),
                    "srv",
                    &catalogs(),
                    authority,
                    McpFeatureOperationOptions::new(Instant::now()),
                    deadline(),
                )
                .await;
            assert!(result.is_err());
            assert!(peer.closed);
            peer.completion().completed().await;
        };
        join(server, client).await;
    });
}

#[test]
fn malformed_or_uncorrelated_feature_response_closes_without_replay() {
    for response in [
        br#"{"jsonrpc":"2.0","id":77,"result":{"resultType":"complete","contents":[]}}"#.as_slice(),
        br#"{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","contents":[{"uri":"test://fixed","text":42}]}}"#.as_slice(),
        br#"{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","contents":[],"contents":[]}}"#.as_slice(),
    ] {
        executor().block_on(async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let selected = options(listener.local_addr().unwrap(), TransportKind::StreamableHttp);
            let server = async { accept_reply(&listener, 200, JSON, &discovery()).await; accept_reply(&listener, 200, JSON, response).await; };
            let client = async {
                let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline()).await.unwrap();
                assert!(peer.feature(&feature_request("resource read srv test://fixed"), "srv", &catalogs(), human(CancellationToken::new()), McpFeatureOperationOptions::new(Instant::now()), deadline()).await.is_err());
                assert!(peer.closed);
                assert_eq!(peer.next_id, Some(3));
            };
            join(server, client).await;
        });
    }
}

#[test]
fn feature_json_and_sse_use_full_bounded_capacity_without_changing_ordinary_limits() {
    for media in [JSON, SSE] {
        executor().block_on(async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let mut selected = options(listener.local_addr().unwrap(), TransportKind::StreamableHttp);
            let until = Instant::now() + Duration::from_secs(45);
            selected.lifetime = until.into();
            let body = format!(r#"{{"jsonrpc":"2.0","id":2,"result":{{"resultType":"complete","contents":[],"large":"{}","nodes":[{}0],"number":-0.000e999999}}}}"#, "x".repeat(9 * 1024 * 1024), "0,".repeat(70_000));
            let wire = if media == SSE { format!("data: {body}\n\n") } else { body };
            let server = async {
                accept_reply(&listener, 200, JSON, &discovery()).await;
                accept_reply(&listener, 200, media, wire.as_bytes()).await;
            };
            let client = async {
                let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), until).await.unwrap();
                let result = peer.feature(&feature_request("resource read srv test://fixed"), "srv", &catalogs(), human(CancellationToken::new()), McpFeatureOperationOptions::new(Instant::now()), until).await.unwrap();
                let McpFeatureReply::Response(result) = result else { panic!() };
                assert!(result.raw_json().get().len() > 8 * 1024 * 1024);
                assert!(result.raw_json().get().contains("-0.000e999999"));
                assert_eq!(peer.response_limits, WireLimits::default());
            };
            join(server, client).await;
        });
    }
}

#[test]
fn lowered_feature_response_bound_rejects_atomically_and_unpolled_call_consumes_no_id() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = options(
            listener.local_addr().unwrap(),
            TransportKind::StreamableHttp,
        );
        let server = async {
            accept_reply(&listener, 200, JSON, &discovery()).await;
            accept_reply(
                &listener,
                200,
                JSON,
                br#"{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","contents":[]}}"#,
            )
            .await;
        };
        let client = async {
            let mut peer = McpHttpPeer::connect(selected, CancellationToken::new(), deadline())
                .await
                .unwrap();
            let catalogs = catalogs();
            let request = feature_request("resource read srv test://fixed");
            drop(peer.feature(
                &request,
                "srv",
                &catalogs,
                human(CancellationToken::new()),
                McpFeatureOperationOptions::new(Instant::now()),
                deadline(),
            ));
            assert_eq!(peer.next_id, Some(2));
            let mut options = McpFeatureOperationOptions::new(Instant::now());
            options.codec.max_response_bytes = 32;
            assert!(
                peer.feature(
                    &request,
                    "srv",
                    &catalogs,
                    human(CancellationToken::new()),
                    options,
                    deadline()
                )
                .await
                .is_err()
            );
            assert!(peer.closed);
            assert_eq!(peer.next_id, Some(3));
        };
        join(server, client).await;
    });
}
