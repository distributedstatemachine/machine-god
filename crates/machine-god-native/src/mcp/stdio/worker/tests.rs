use super::super::Queued;
use super::*;
use crate::mcp::protocol::WireLimits;

fn guarded_control(
    token: CancellationToken,
    large: bool,
    retired: Arc<std::sync::atomic::AtomicBool>,
) -> McpStdioControl {
    use crate::mcp::{
        control::tests::{catalogs, human_selected, request},
        feature::{McpFeatureCodecLimits, McpFeatureExchange, McpFeatureExchangeOptions},
        peer::McpPeerCapabilities,
        protocol::{NegotiatedProtocol, ProtocolVersion, TransportKind, parse_envelope},
    };
    let envelope = parse_envelope(br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","capabilities":{"prompts":{}}}}"#, WireLimits::default()).unwrap();
    let capabilities = McpPeerCapabilities::admit(&envelope, ProtocolVersion::Modern).unwrap();
    let command = format!(
        r#"prompt get srv review {{"topic":"{}"}}"#,
        "x".repeat(if large { 32 * 1024 } else { 1 })
    );
    let exchange = McpFeatureExchange::prepare(
        &request(&command),
        "srv",
        &catalogs(),
        McpFeatureExchangeOptions::new(
            NegotiatedProtocol {
                version: ProtocolVersion::Modern,
                transport: TransportKind::Stdio,
            },
            7,
            capabilities,
        )
        .unwrap(),
        None,
        McpFeatureCodecLimits::default(),
    )
    .unwrap();
    McpStdioControl::feature(&exchange, human_selected(token, retired)).unwrap()
}

#[test]
fn typed_control_cancellation_blocks_initial_write_suffix_and_final_completion() {
    for case in 0..6 {
        let stage = case % 3;
        let token = CancellationToken::new();
        let retired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let control = guarded_control(token.clone(), stage == 1, retired.clone());
        let length = control.bytes.len();
        let (input, output) = UnixStream::pair().unwrap();
        input.set_nonblocking(true).unwrap();
        output.set_nonblocking(true).unwrap();
        let input = Arc::new(input);
        let mut active = Active {
            write: Write::Control {
                control,
                offset: 0,
                attempted: false,
            },
            deadline: Instant::now() + Duration::from_secs(5),
            cancel: CancellationToken::new(),
            connection: CancellationToken::new(),
            host: CancellationToken::new(),
            response: Arc::new(Response::new()),
        };
        let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
        let mut received = vec![0; READ_BYTES];
        if stage != 0 {
            assert!(active.poll(&input, &mut cx).is_pending());
            let acknowledged = active.receipt(Ok(())).acknowledged_bytes;
            assert!(acknowledged > 0 && acknowledged <= length.min(READ_BYTES));
            if stage == 2 {
                assert_eq!(acknowledged, length);
            }
            let mut count = 0;
            while count < acknowledged {
                count += rustix::io::read(&output, &mut received[count..acknowledged]).unwrap();
            }
        }
        if case < 3 {
            token.cancel();
        } else {
            retired.store(true, std::sync::atomic::Ordering::Release);
            assert!(!token.is_cancelled());
        }
        assert!(matches!(
            active.poll(&input, &mut cx),
            Poll::Ready(Err(McpStdioError::Cancelled))
        ));
        assert_eq!(
            rustix::io::read(&output, &mut received),
            Err(rustix::io::Errno::AGAIN)
        );
    }
}

#[test]
fn queued_typed_control_retirement_is_pruned_without_write() {
    let token = CancellationToken::new();
    let control = guarded_control(
        token.clone(),
        false,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    let shared = Shared::new(WireLimits::default(), CancellationToken::new());
    let response = Arc::new(Response::new());
    {
        let mut state = shared.state.lock().unwrap();
        state.admitted = 1;
        state.queue.push_back(Queued {
            payload: Payload::Control(control),
            deadline: Instant::now() + Duration::from_secs(5),
            cancel: CancellationToken::new(),
            response: response.clone(),
        });
    }
    token.cancel();
    prune(
        &shared,
        &mut Context::from_waker(futures_util::task::noop_waker_ref()),
    )
    .unwrap();
    assert!(shared.state.lock().unwrap().queue.is_empty());
    let receipt = futures_executor::block_on(response.wait()).unwrap();
    assert_eq!(receipt.outcome, Err(McpStdioError::Cancelled));
    assert!(!receipt.attempted);
}

#[test]
fn partial_cancel_closes_before_any_next_frame_and_preserves_evidence() {
    let (input, peer) = UnixStream::pair().unwrap();
    input.set_nonblocking(true).unwrap();
    peer.set_nonblocking(true).unwrap();
    rustix::net::sockopt::set_socket_send_buffer_size(&input, 1024).unwrap();
    let input = Arc::new(input);
    let bytes = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{{"padding":"{}"}}}}"#,
        "a".repeat(100_000)
    );
    let response = Arc::new(Response::new());
    let cancellation = CancellationToken::new();
    let mut active = Active {
        write: Write::Control {
            control: McpStdioControl::discovery(bytes.as_bytes()).unwrap(),
            offset: 0,
            attempted: false,
        },
        deadline: Instant::now() + Duration::from_secs(5),
        cancel: cancellation.clone(),
        connection: CancellationToken::new(),
        host: CancellationToken::new(),
        response,
    };
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    for _ in 0..128 {
        assert!(active.poll(&input, &mut cx).is_pending());
    }
    let before = active.receipt(Ok(()));
    assert!(before.acknowledged_bytes > 0 && before.acknowledged_bytes < bytes.len());
    let shared = Shared::new(WireLimits::default(), CancellationToken::new());
    let next = Arc::new(Response::new());
    {
        let mut state = shared.state.lock().unwrap();
        state.admitted = 2;
        state.queue.push_back(Queued {
            payload: Payload::Control(
                McpStdioControl::discovery(
                    br#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"marker":"NEXT"}}"#,
                )
                .unwrap(),
            ),
            deadline: Instant::now() + Duration::from_secs(5),
            cancel: CancellationToken::new(),
            response: next.clone(),
        });
    }
    let (output, _writer) = std::io::pipe().unwrap();
    let flags = rustix::fs::fcntl_getfl(&output).unwrap();
    rustix::fs::fcntl_setfl(&output, flags | rustix::fs::OFlags::NONBLOCK).unwrap();
    cancellation.cancel();
    let mut active = Some(active);
    assert_eq!(
        run_io(
            &input,
            &output,
            &shared,
            &CancellationToken::new(),
            &mut active,
            || {},
        ),
        McpStdioError::Cancelled
    );
    assert!(active.is_none());
    shared.finish(McpStdioError::Cancelled);
    let next = futures_executor::block_on(next.wait()).unwrap();
    assert!(!next.attempted);
    let mut received = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        match rustix::io::read(&peer, &mut chunk) {
            Ok(count) if count > 0 => received.extend_from_slice(&chunk[..count]),
            Err(rustix::io::Errno::AGAIN) => break,
            other => panic!("unexpected read: {other:?}"),
        }
    }
    assert_eq!(received, bytes.as_bytes()[..before.acknowledged_bytes]);
    assert!(!received.windows(4).any(|chunk| chunk == b"NEXT"));
}

#[test]
fn bounded_framing_eof_and_overflow_fan_out_without_processes() {
    use std::io::Write as _;
    for (bytes, limit, expected) in [
        (b"{".as_slice(), 64, McpStdioError::Protocol),
        (b"123456789\n", 8, McpStdioError::Protocol),
        (b"", 64, McpStdioError::Closed),
    ] {
        let (input, _peer) = UnixStream::pair().unwrap();
        let (output, mut writer) = std::io::pipe().unwrap();
        writer.write_all(bytes).unwrap();
        drop(writer);
        let shared = Shared::new(
            WireLimits {
                max_frame_bytes: limit,
                ..WireLimits::default()
            },
            CancellationToken::new(),
        );
        let mut active = None;
        let error = run_io(
            &Arc::new(input),
            &output,
            &shared,
            &CancellationToken::new(),
            &mut active,
            || {},
        );
        assert_eq!(error, expected);
        shared.finish(error);
        assert_eq!(shared.state.lock().unwrap().closed, Some(expected));
    }
}

#[test]
fn queued_turn_cancellation_is_observed_without_writer_permission_or_io() {
    let fixture = crate::mcp::submission::tests::Fixture::new();
    fixture.ready("call");
    let cancellation = CancellationToken::new();
    let submission =
        futures_executor::block_on(fixture.claim("call", cancellation.clone())).unwrap();
    let shared = Shared::new(WireLimits::default(), CancellationToken::new());
    let response = Arc::new(Response::new());
    {
        let mut state = shared.state.lock().unwrap();
        state.admitted = 1;
        state.queue.push_back(Queued {
            payload: Payload::Tool(submission),
            deadline: Instant::now() + Duration::from_secs(5),
            cancel: CancellationToken::new(),
            response: response.clone(),
        });
    }
    let _ = fixture.turn_handle().cancel();
    prune(
        &shared,
        &mut Context::from_waker(futures_util::task::noop_waker_ref()),
    )
    .unwrap();
    let receipt = futures_executor::block_on(response.wait()).unwrap();
    assert_eq!(receipt.outcome, Err(McpStdioError::Cancelled));
    assert!(!receipt.attempted);
    assert_eq!(shared.state.lock().unwrap().admitted, 0);
}

#[test]
fn proof_revocation_and_final_pipe_stop_prevent_bytes() {
    let fixture = crate::mcp::submission::tests::Fixture::new();
    fixture.ready("revoked");
    let submission =
        futures_executor::block_on(fixture.claim("revoked", CancellationToken::new())).unwrap();
    let (input, peer) = UnixStream::pair().unwrap();
    input.set_nonblocking(true).unwrap();
    peer.set_nonblocking(true).unwrap();
    let stop = CancellationToken::new();
    let input = Arc::new(input);
    let make_writer = || SocketWriter {
        feature: None,
        input: input.clone(),
        connection: stop.clone(),
        host: CancellationToken::new(),
        request: CancellationToken::new(),
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let mut write = submission.into_writer(make_writer());
    fixture.revoke();
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(matches!(
        Pin::new(&mut write).poll(&mut cx),
        Poll::Ready(Err(_))
    ));
    assert!(!write.was_attempted());
    stop.cancel();
    assert!(matches!(
        make_writer().poll_write(&mut cx, b"unsubmitted"),
        Poll::Ready(Err(_))
    ));
    assert!(matches!(
        rustix::io::read(&peer, &mut [0; 1]),
        Err(rustix::io::Errno::AGAIN)
    ));
}

#[test]
fn response_burst_retains_one_read_tail_and_resumes_after_sequential_drains() {
    use std::io::Write as _;
    let (output, mut writer) = std::io::pipe().unwrap();
    for id in 1..=4 {
        writeln!(
            writer,
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{}}}}"
        )
        .unwrap();
    }
    drop(writer);
    let shared = Arc::new(Shared::new(WireLimits::default(), CancellationToken::new()));
    let scope = crate::NativeOwnedWorkerScope::new();
    let connection = crate::mcp::stdio::McpStdioConnection {
        shared: shared.clone(),
        completion: scope.completion(),
    };
    let mut reader = ReadState {
        decoder: NdjsonDecoder::new(shared.limits).unwrap(),
        shared: &shared,
        bytes: [0; READ_BYTES],
        start: 0,
        end: 0,
    };
    assert_eq!(read_once(&output, &mut reader), Ok(true));
    assert_eq!(
        shared.state.lock().unwrap().frames.len(),
        MAX_MCP_STDIO_FRAMES
    );
    assert!(reader.start < reader.end);
    assert_eq!(read_once(&output, &mut reader), Ok(false));
    for expected in 1..=4 {
        let envelope = futures_executor::block_on(connection.receive()).unwrap();
        assert_eq!(
            envelope.id(),
            Some(&crate::mcp::protocol::RpcId::Integer(expected))
        );
        let result = read_once(&output, &mut reader);
        assert!(result.is_ok() || result == Err(McpStdioError::Closed));
    }
    assert_eq!(read_once(&output, &mut reader), Err(McpStdioError::Closed));
    assert_eq!(
        shared.state.lock().unwrap().read_end,
        Some(McpStdioReadEnd::CleanEof)
    );
}

#[test]
fn partial_buffer_on_cancellation_is_not_clean_eof_or_timeout_evidence() {
    let shared = Shared::new(WireLimits::default(), CancellationToken::new());
    {
        let mut reader = ReadState {
            decoder: NdjsonDecoder::new(shared.limits).unwrap(),
            shared: &shared,
            bytes: [0; READ_BYTES],
            start: 0,
            end: 0,
        };
        reader.decoder.push(b"{\"jsonrpc\":").unwrap();
        shared.stop.cancel();
    }
    let state = shared.state.lock().unwrap();
    assert_eq!(state.read_end, Some(McpStdioReadEnd::Unclassified));
    assert!(state.buffered_partial_frame);
}
#[test]
fn process_handoff_occurs_only_after_publication_and_before_cancel_cleanup() {
    for published in [false, true] {
        let shared = Shared::new(WireLimits::default(), CancellationToken::new());
        if published {
            shared.handoff.publish();
        }
        shared.stop.cancel();
        let (input, _peer) = UnixStream::pair().unwrap();
        let (output, _writer) = std::io::pipe().unwrap();
        let mut active = None;
        let mut transfers = 0;
        assert_eq!(
            run_io(
                &Arc::new(input),
                &output,
                &shared,
                &CancellationToken::new(),
                &mut active,
                || transfers += 1,
            ),
            McpStdioError::Cancelled
        );
        assert_eq!(transfers, usize::from(published));
    }
}
