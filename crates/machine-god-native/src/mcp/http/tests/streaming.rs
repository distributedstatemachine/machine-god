use super::*;

#[test]
fn event_stream_bytes_feed_the_existing_bounded_sse_decoder() {
    executor().block_on(async {
        use crate::mcp::sse::{SseDecoder, SseLimits, SseMode};
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let client = connection(listener.local_addr().unwrap(), CancellationToken::new());
        let observation = client.observation();
        let server = async {
            serve(listener.accept().await.unwrap().0,
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: {\"ok\":true}\n\n").await
        };
        let client = async {
            let mut response = client.control(McpHttpControl::listen()).await.unwrap();
            assert_eq!(response.headers.iter().collect::<Vec<_>>(), vec![("content-type", &b"text/event-stream"[..])]);
            let mut decoder = SseDecoder::new(SseMode::Modern, SseLimits::default()).unwrap();
            let mut events = Vec::new();
            while let Some(bytes) = response.body.next_chunk().await.unwrap() {
                let mut remaining = bytes.as_ref();
                while !remaining.is_empty() {
                    let progress = decoder.push(remaining).unwrap();
                    remaining = &remaining[progress.consumed..];
                    if let Some(event) = progress.event { events.push(event); }
                }
            }
            decoder.finish().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].data(), "{\"ok\":true}");
        };
        let ((), written) = join(client, server).await;
        assert!(written.starts_with(b"GET /mcp?private=query HTTP/1.1\r\n"));
        assert!(observation.is_complete());
    });
}

#[test]
fn deadline_covers_pending_response_head_and_body() {
    executor().block_on(async {
        for head in [false, true] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let mut client = connection(listener.local_addr().unwrap(), CancellationToken::new());
            client.lifetime = super::super::io::Lifetime::new(
                CancellationToken::new(),
                Instant::now() + Duration::from_millis(200),
            );
            let observation = client.observation();
            let server = async {
                let (mut stream, _) = listener.accept().await.unwrap();
                request(&mut stream).await;
                if head {
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n")
                        .await
                        .unwrap();
                }
                assert_eq!(stream.read(&mut [0]).await.unwrap(), 0);
            };
            let client = async {
                let response = client.control(McpHttpControl::listen()).await;
                if head {
                    assert_eq!(
                        response.unwrap().body.next_chunk().await,
                        Err(McpHttpError::Deadline)
                    );
                } else {
                    assert_eq!(response.unwrap_err(), McpHttpError::Deadline);
                }
            };
            join(client, server).await;
            assert!(observation.is_complete());
        }
    });
}

#[test]
fn response_head_body_and_wire_budgets_close_at_the_boundary() {
    executor().block_on(async {
        for (limits, wire) in [
            (
                McpHttpLimits {
                    head_bytes: 16,
                    ..McpHttpLimits::default()
                },
                &b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"[..],
            ),
            (
                McpHttpLimits {
                    header_count: 1,
                    ..McpHttpLimits::default()
                },
                &b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nX-Test: value\r\n\r\n"[..],
            ),
            (
                McpHttpLimits {
                    body_bytes: 2,
                    ..McpHttpLimits::default()
                },
                &b"HTTP/1.1 200 OK\r\n\r\nthree"[..],
            ),
            (
                McpHttpLimits {
                    wire_bytes: 8,
                    ..McpHttpLimits::default()
                },
                &b"HTTP/1.1 200 OK\r\n\r\n"[..],
            ),
        ] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let mut client = connection(listener.local_addr().unwrap(), CancellationToken::new());
            client.limits = limits;
            let observation = client.observation();
            let server = async { serve(listener.accept().await.unwrap().0, wire).await };
            let client = async {
                let result = client.control(McpHttpControl::listen()).await;
                if let Ok(mut response) = result {
                    assert!(collect(&mut response.body).await.is_err());
                }
            };
            join(client, server).await;
            assert!(observation.is_complete());
        }
    });
}

#[test]
fn original_tool_turn_cancellation_remains_live_during_response_reads() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let client = connection(listener.local_addr().unwrap(), CancellationToken::new());
        let observation = client.observation();
        let fixture = crate::mcp::submission::tests::Fixture::new();
        fixture.ready_http("response-cancel", &client.head);
        let submission = fixture
            .claim("response-cancel", CancellationToken::new())
            .await
            .unwrap();
        let server = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            request(&mut stream).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n")
                .await
                .unwrap();
            assert_eq!(stream.read(&mut [0]).await.unwrap(), 0);
        };
        let client = async {
            let mut response = client
                .submit(submission, fixture.runtime.clone())
                .await
                .unwrap();
            assert!(fixture.turn_handle().cancel());
            assert_eq!(
                response.body.next_chunk().await,
                Err(McpHttpError::Cancelled)
            );
        };
        join(client, server).await;
        assert!(observation.is_complete());
    });
}
