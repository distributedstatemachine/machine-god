use super::*;
use crate::mcp::submission::{McpSubmissionHttpHead, tests::Fixture};

#[test]
fn actual_proof_bearing_exchange_sends_only_exact_admitted_request() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let client = connection(listener.local_addr().unwrap(), CancellationToken::new());
        let observation = client.observation();
        let fixture = Fixture::new();
        fixture.ready_http("http-call", &client.head);
        let submission = fixture
            .claim("http-call", CancellationToken::new())
            .await
            .unwrap();
        let expected = submission.http_request_bytes().unwrap().to_vec();
        let server = async {
            serve(
                listener.accept().await.unwrap().0,
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}",
            )
            .await
        };
        let client = async {
            let mut response = client
                .submit(submission, fixture.runtime.clone())
                .await
                .unwrap();
            assert_eq!(
                response
                    .body
                    .promote_listener(crate::mcp::lifetime::McpPeerLifetime::OwnerControlled),
                Err(McpHttpError::Invalid)
            );
            assert_eq!(collect(&mut response.body).await.unwrap(), b"{}");
        };
        let ((), written) = join(client, server).await;
        assert_eq!(written, expected);
        assert_eq!(observation.acknowledged_bytes(), expected.len());
        assert!(observation.is_complete());
    });
}

#[test]
fn mismatched_head_and_foreign_runtime_are_rejected_before_connect() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let fixture = Fixture::new();
        let other = Fixture::new();
        for foreign in [false, true] {
            let client = connection(listener.local_addr().unwrap(), CancellationToken::new());
            let head = if foreign {
                McpSubmissionHttpHead::new(
                    client.endpoint(),
                    &[("Authorization", b"Bearer private-value")],
                )
                .unwrap()
            } else {
                McpSubmissionHttpHead::new(
                    client.endpoint(),
                    &[("Authorization", b"Bearer different")],
                )
                .unwrap()
            };
            let call = if foreign { "foreign" } else { "changed-head" };
            fixture.ready_http(call, &head);
            let submission = fixture.claim(call, CancellationToken::new()).await.unwrap();
            let runtime = if foreign {
                other.runtime.clone()
            } else {
                fixture.runtime.clone()
            };
            let observation = client.observation();
            assert_eq!(
                client.submit(submission, runtime).await.unwrap_err(),
                McpHttpError::Submission
            );
            assert!(!observation.was_attempted());
            assert!(observation.is_complete());
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
    });
}

#[test]
fn cancellation_after_real_partial_plaintext_write_preserves_attempt_and_never_replays() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let cancel = CancellationToken::new();
        let original = connection(listener.local_addr().unwrap(), cancel.clone());
        let endpoint = original.endpoint().clone();
        drop(original);
        let values: Vec<_> = (0..32)
            .map(|index| (format!("x-pad-{index}"), vec![b'p'; 16 * 1024]))
            .collect();
        let headers: Vec<_> = values
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_slice()))
            .collect();
        let client = McpHttpConnection::new(
            McpHttpDestination::new(endpoint, &[listener.local_addr().unwrap()]).unwrap(),
            &headers,
            None,
            McpHttpLimits::default(),
            cancel.clone(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        let observation = client.observation();
        let fixture = Fixture::new();
        fixture.ready_http("partial", &client.head);
        let submission = fixture
            .claim("partial", CancellationToken::new())
            .await
            .unwrap();
        let size = submission.http_request_bytes().unwrap().len();
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut part = [0; 1024];
            assert!(socket.read(&mut part).await.unwrap() > 0);
            cancel.cancel();
            let mut tail = Vec::new();
            let _ = socket.read_to_end(&mut tail).await;
        };
        let client = async {
            assert!(
                client
                    .submit(submission, fixture.runtime.clone())
                    .await
                    .is_err()
            );
        };
        join(client, server).await;
        assert!(observation.was_attempted());
        assert!(observation.acknowledged_bytes() > 0);
        assert!(observation.acknowledged_bytes() < size);
        assert!(observation.is_complete());
        assert!(
            fixture
                .claim("partial", CancellationToken::new())
                .await
                .is_err()
        );
    });
}
