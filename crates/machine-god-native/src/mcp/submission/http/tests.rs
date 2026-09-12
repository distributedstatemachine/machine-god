use super::*;
use crate::mcp::endpoint::McpEndpoint;
use std::collections::VecDeque;
use std::io::IoSlice;

fn head() -> McpSubmissionHttpHead {
    McpSubmissionHttpHead::new(
        &McpEndpoint::parse("https://example.test:8443/mcp?secret=query").unwrap(),
        &[
            ("Authorization", b"Bearer secret-auth"),
            ("Mcp-Protocol-Version", b"2026-07-28"),
        ],
    )
    .unwrap()
}
fn prepared(
    fixture: &Fixture,
    call: &str,
    cancellation: CancellationToken,
) -> PreparedMcpSubmission {
    let request = fixture.request(call);
    let Capability::Tool {
        name,
        call_id,
        arguments,
    } = &request.capability
    else {
        panic!()
    };
    block_on(fixture.registry.prepare_http(
        &request,
        PermissionInvocation {
            tool_name: name,
            call_id,
            arguments,
        },
        fixture.runtime.clone(),
        &head(),
        &fixture.wire(),
        cancellation,
    ))
    .unwrap()
}
fn submission(
    fixture: &Fixture,
    preparation: CancellationToken,
    execution: CancellationToken,
) -> McpSubmission {
    fixture
        .admission("call", prepared(fixture, "call", preparation))
        .admit()
        .unwrap();
    block_on(fixture.claim("call", execution)).unwrap()
}
fn driver(fixture: &Fixture, sink: Sink) -> McpSubmissionHttpDriver<Sink> {
    submission(fixture, CancellationToken::new(), CancellationToken::new())
        .into_http_driver(sink)
        .unwrap()
}
fn context() -> Context<'static> {
    Context::from_waker(futures_util::task::noop_waker_ref())
}

#[test]
fn connector_observes_exact_http_bytes_without_claiming_write_authority() {
    let fixture = Fixture::new();
    fixture.ready_http("http", &head());
    let submission = block_on(fixture.claim("http", CancellationToken::new())).unwrap();
    assert_eq!(
        submission.http_request_bytes().unwrap(),
        head().encode(&fixture.wire()).unwrap().as_ref()
    );
    assert!(!submission.was_attempted());
    fixture.ready("stdio");
    let stdio = block_on(fixture.claim("stdio", CancellationToken::new())).unwrap();
    assert_eq!(stdio.http_request_bytes(), Err(McpSubmissionError::Invalid));
    assert!(!stdio.was_attempted());
}

#[derive(Clone, Copy)]
enum Step {
    Accept(usize),
    Pending,
    Fail,
    Zero,
    InvalidCount,
    Panic,
}
#[derive(Default)]
struct SinkState {
    bytes: Vec<u8>,
    scalar: usize,
    vector: usize,
    flush: usize,
    drops: usize,
    writes: VecDeque<Step>,
    flushes: VecDeque<Step>,
    callback: Option<Box<dyn FnOnce() + Send>>,
}
struct Sink(Arc<Mutex<SinkState>>);
impl Sink {
    fn write(&self, chunks: &[IoSlice<'_>], vector: bool) -> Poll<io::Result<usize>> {
        let (step, callback) = {
            let mut state = self.0.lock().unwrap();
            if vector {
                state.vector += 1;
            } else {
                state.scalar += 1;
            }
            (
                state.writes.pop_front().unwrap_or(Step::Accept(usize::MAX)),
                state.callback.take(),
            )
        };
        if let Some(callback) = callback {
            callback();
        }
        let offered: usize = chunks.iter().map(|chunk| chunk.len()).sum();
        match step {
            Step::Pending => Poll::Pending,
            Step::Zero => Poll::Ready(Ok(0)),
            Step::InvalidCount => Poll::Ready(Ok(offered + 1)),
            Step::Panic => panic!("controlled writer panic"),
            Step::Accept(limit) => {
                let count = offered.min(limit);
                self.append(chunks, count);
                Poll::Ready(Ok(count))
            }
            Step::Fail => {
                self.append(chunks, offered.min(1));
                Poll::Ready(Err(io::Error::other("secret sink failure")))
            }
        }
    }
    fn append(&self, chunks: &[IoSlice<'_>], mut count: usize) {
        let mut state = self.0.lock().unwrap();
        for chunk in chunks {
            let take = count.min(chunk.len());
            state.bytes.extend_from_slice(&chunk[..take]);
            count -= take;
        }
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        self.0.lock().unwrap().drops += 1;
    }
}
impl McpSubmissionWriter for Sink {
    fn poll_write(&mut self, _: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        self.write(&[IoSlice::new(bytes)], false)
    }
    fn poll_write_vectored(
        &mut self,
        _: &mut Context<'_>,
        bytes: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.write(bytes, true)
    }
    fn poll_flush(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        let (step, callback) = {
            let mut state = self.0.lock().unwrap();
            state.flush += 1;
            (
                state.flushes.pop_front().unwrap_or(Step::Accept(0)),
                state.callback.take(),
            )
        };
        if let Some(callback) = callback {
            callback();
        }
        match step {
            Step::Pending => Poll::Pending,
            Step::Panic => panic!("controlled flush panic"),
            Step::Fail | Step::Zero | Step::InvalidCount => {
                Poll::Ready(Err(io::Error::other("secret flush failure")))
            }
            Step::Accept(_) => Poll::Ready(Ok(())),
        }
    }
}

#[test]
fn http_frame_is_fixed_before_admission_and_has_no_ndjson_newline() {
    let fixture = Fixture::new();
    let state = Arc::new(Mutex::new(SinkState::default()));
    let mut driver = driver(&fixture, Sink(state.clone()));
    let request = driver.request_bytes().to_vec();
    let body = fixture.wire();
    let expected = format!(
        "POST /mcp?secret=query HTTP/1.1\r\nhost: example.test:8443\r\ncontent-type: application/json\r\naccept: application/json, text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\nauthorization: Bearer secret-auth\r\nmcp-protocol-version: 2026-07-28\r\n\r\n",
        body.len()
    );
    assert_eq!(&request[..expected.len()], expected.as_bytes());
    assert_eq!(&request[expected.len()..], body);
    assert!(!driver.was_attempted());
    assert_eq!(state.lock().unwrap().scalar, 0);
    assert_eq!(
        driver.poll_write(&mut context(), &request),
        Poll::Ready(Ok(request.len()))
    );
    assert!(!driver.is_complete());
    assert_eq!(driver.poll_flush(&mut context()), Poll::Ready(Ok(())));
    assert!(driver.is_complete());
    assert_eq!(state.lock().unwrap().bytes, request);
    assert!(!format!("{driver:?} {:?}", head()).contains("secret"));
}

#[test]
fn header_conflicts_smuggling_controls_duplicates_and_bounds_are_rejected() {
    let endpoint = McpEndpoint::parse("https://example.test/mcp").unwrap();
    for name in [
        "HOST",
        "Content-Length",
        "Connection",
        "Accept",
        "Content-Type",
        "Transfer-Encoding",
        "Trailer",
        "Upgrade",
        "Expect",
        "TE",
        "Proxy-Connection",
        "Content-Encoding",
        "Proxy-Authorization",
    ] {
        assert!(McpSubmissionHttpHead::new(&endpoint, &[(name, b"secret")]).is_err());
    }
    for fields in [
        vec![("x-ok", b"a".as_slice()), ("X-OK", b"b")],
        vec![("x\r\ninjected", b"value")],
        vec![("x-ok", b"a\r\nb: c")],
        vec![("x-ok", b"a\x7fb")],
        vec![("x-ok", b"a\x00b")],
    ] {
        assert!(McpSubmissionHttpHead::new(&endpoint, &fields).is_err());
    }
    assert!(matches!(
        McpSubmissionHttpHead::new(&endpoint, &vec![("x", b"".as_slice()); 257]),
        Err(McpSubmissionError::Limit)
    ));
    assert!(matches!(
        McpSubmissionHttpHead::new(&endpoint, &[(&"n".repeat(16 * 1024 + 1), b"")]),
        Err(McpSubmissionError::Limit)
    ));
    assert!(McpSubmissionHttpHead::new(&endpoint, &[("x", &vec![b'v'; 16 * 1024 + 1])]).is_err());
}

fn boundary_headers(count: usize, field_bytes: usize) -> (Vec<String>, Vec<Vec<u8>>) {
    let mut names: Vec<_> = (0..count).map(|index| format!("x-{index:03}")).collect();
    names[0] = "n".repeat(16 * 1024);
    let mut remaining = field_bytes - names.iter().map(String::len).sum::<usize>();
    let values = (0..count)
        .map(|_| {
            let count = remaining.min(16 * 1024);
            remaining -= count;
            vec![b'v'; count]
        })
        .collect();
    assert_eq!(remaining, 0);
    (names, values)
}

fn prepare_with_head(fixture: &Fixture, head: &McpSubmissionHttpHead) -> PreparedMcpSubmission {
    let request = fixture.request("call");
    let Capability::Tool {
        name,
        call_id,
        arguments,
    } = &request.capability
    else {
        panic!()
    };
    block_on(fixture.registry.prepare_http(
        &request,
        PermissionInvocation {
            tool_name: name,
            call_id,
            arguments,
        },
        fixture.runtime.clone(),
        head,
        &fixture.wire(),
        CancellationToken::new(),
    ))
    .unwrap()
}

#[test]
fn maximal_resolved_headers_plus_protocol_fields_fit_the_transport_head() {
    let fixture = Fixture::new();
    let endpoint =
        McpEndpoint::parse(&format!("https://example.test/{}", "p".repeat(4000))).unwrap();
    let (names, values) = boundary_headers(128, 512 * 1024);
    let mut borrowed: Vec<_> = names
        .iter()
        .zip(&values)
        .map(|(name, value)| (name.as_str(), value.as_slice()))
        .collect();
    assert_eq!(borrowed.len(), 128);
    assert_eq!(
        borrowed
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum::<usize>(),
        512 * 1024
    );
    borrowed.extend([
        ("mcp-protocol-version", b"2026-07-28".as_slice()),
        ("mcp-method", b"tools/call"),
    ]);
    let head = McpSubmissionHttpHead::new(&endpoint, &borrowed).unwrap();
    fixture
        .admission("call", prepare_with_head(&fixture, &head))
        .admit()
        .unwrap();
    let state = Arc::new(Mutex::new(SinkState::default()));
    let mut driver = block_on(fixture.claim("call", CancellationToken::new()))
        .unwrap()
        .into_http_driver(Sink(state.clone()))
        .unwrap();
    let wire = driver.request_bytes().to_vec();
    assert!(wire.len() > 512 * 1024 + endpoint.request_target().len());
    assert!(wire.len() < 1024 * 1024 + MAX_MCP_SUBMISSION_REQUEST_BYTES);
    assert_eq!(
        driver.poll_write(&mut context(), &wire),
        Poll::Ready(Ok(wire.len()))
    );
    assert_eq!(driver.poll_flush(&mut context()), Poll::Ready(Ok(())));
    assert_eq!(state.lock().unwrap().bytes, wire);
}

#[test]
fn composed_header_count_name_value_and_aggregate_bounds_are_inclusive() {
    let fixture = Fixture::new();
    let endpoint = McpEndpoint::parse("https://example.test/mcp").unwrap();
    let (names, mut values) = boundary_headers(256, 768 * 1024);
    let borrowed: Vec<_> = names
        .iter()
        .zip(&values)
        .map(|(name, value)| (name.as_str(), value.as_slice()))
        .collect();
    assert_eq!(borrowed.len(), 256);
    assert_eq!(borrowed[0].0.len(), 16 * 1024);
    assert_eq!(borrowed[0].1.len(), 16 * 1024);
    assert_eq!(
        borrowed
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum::<usize>(),
        768 * 1024
    );
    let head = McpSubmissionHttpHead::new(&endpoint, &borrowed).unwrap();
    let prepared = prepare_with_head(&fixture, &head);
    assert!(prepared.data.wire.len() > 768 * 1024);
    assert!(prepared.data.wire.len() < 1024 * 1024 + MAX_MCP_SUBMISSION_REQUEST_BYTES);
    drop(borrowed);
    values.last_mut().unwrap().push(b'v');
    let borrowed: Vec<_> = names
        .iter()
        .zip(&values)
        .map(|(name, value)| (name.as_str(), value.as_slice()))
        .collect();
    assert!(matches!(
        McpSubmissionHttpHead::new(&endpoint, &borrowed),
        Err(McpSubmissionError::Limit)
    ));
}

#[test]
fn resolved_non_utf8_and_tab_header_bytes_survive_preparation_exactly() {
    let fixture = Fixture::new();
    let endpoint = McpEndpoint::parse("https://example.test/mcp").unwrap();
    let head = McpSubmissionHttpHead::new(&endpoint, &[("X-Bytes", b"\tsecret-\x80\xff")]).unwrap();
    let request = fixture.request("call");
    let Capability::Tool {
        name,
        call_id,
        arguments,
    } = &request.capability
    else {
        panic!()
    };
    let prepared = block_on(fixture.registry.prepare_http(
        &request,
        PermissionInvocation {
            tool_name: name,
            call_id,
            arguments,
        },
        fixture.runtime.clone(),
        &head,
        &fixture.wire(),
        CancellationToken::new(),
    ))
    .unwrap();
    fixture.admission("call", prepared).admit().unwrap();
    let mut driver = block_on(fixture.claim("call", CancellationToken::new()))
        .unwrap()
        .into_http_driver(Sink(Arc::default()))
        .unwrap();
    let bytes = driver.request_bytes().to_vec();
    let header = b"x-bytes: \tsecret-\x80\xff\r\n";
    assert!(bytes.windows(header.len()).any(|chunk| chunk == header));
    assert_eq!(
        driver.poll_write(&mut context(), &bytes),
        Poll::Ready(Ok(bytes.len()))
    );
    assert_eq!(driver.poll_flush(&mut context()), Poll::Ready(Ok(())));
}

#[test]
fn final_flush_errors_and_panics_are_terminal_even_after_all_bytes_were_accepted() {
    for step in [Step::Fail, Step::Panic] {
        let fixture = Fixture::new();
        let state = Arc::new(Mutex::new(SinkState {
            flushes: [step].into(),
            ..SinkState::default()
        }));
        let mut driver = driver(&fixture, Sink(state.clone()));
        let bytes = driver.request_bytes().to_vec();
        assert_eq!(
            driver.poll_write(&mut context(), &bytes),
            Poll::Ready(Ok(bytes.len()))
        );
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            driver.poll_flush(&mut context())
        }));
        if matches!(step, Step::Panic) {
            assert!(outcome.is_err());
        } else {
            assert_eq!(
                outcome.unwrap(),
                Poll::Ready(Err(McpSubmissionError::WriterFailed))
            );
        }
        assert_eq!(driver.acknowledged_bytes(), bytes.len());
        assert!(!driver.is_complete());
        assert!(driver.was_attempted());
        assert_eq!(
            driver.poll_flush(&mut context()),
            Poll::Ready(Err(McpSubmissionError::AlreadyAttempted))
        );
        assert_eq!(state.lock().unwrap().flush, 1);
    }
}

#[test]
fn cancelled_or_retired_unpolled_driver_never_enters_its_sink() {
    for source in 0..5 {
        let mut fixture = Fixture::new();
        let state = Arc::new(Mutex::new(SinkState::default()));
        let preparation = CancellationToken::new();
        let execution = CancellationToken::new();
        let mut driver = submission(&fixture, preparation.clone(), execution.clone())
            .into_http_driver(Sink(state.clone()))
            .unwrap();
        let bytes = driver.request_bytes().to_vec();
        match source {
            0 => {
                preparation.cancel();
            }
            1 => {
                execution.cancel();
            }
            2 => {
                assert!(fixture.turn_handle().cancel());
            }
            3 => {
                fixture.close();
            }
            _ => {
                fixture.runtime_owner.retire();
            }
        }
        assert_eq!(
            driver.poll_write_vectored(&mut context(), &[IoSlice::new(&bytes)]),
            Poll::Ready(Err(McpSubmissionError::Cancelled))
        );
        assert!(!driver.was_attempted());
        assert_eq!(state.lock().unwrap().vector, 0);
    }
}

#[test]
fn preparation_is_inert_precancelled_and_transport_modes_do_not_interchange() {
    let fixture = Fixture::new();
    let request = fixture.request("call");
    let Capability::Tool {
        name,
        call_id,
        arguments,
    } = &request.capability
    else {
        panic!()
    };
    let cancellation = CancellationToken::new();
    let future = fixture.registry.prepare_http(
        &request,
        PermissionInvocation {
            tool_name: name,
            call_id,
            arguments,
        },
        fixture.runtime.clone(),
        &head(),
        &fixture.wire(),
        cancellation.clone(),
    );
    assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
    cancellation.cancel();
    assert!(matches!(
        block_on(future),
        Err(McpSubmissionError::Cancelled)
    ));
    assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
    fixture.ready("call");
    let state = Arc::new(Mutex::new(SinkState::default()));
    let claimed = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
    assert!(matches!(
        claimed.into_http_driver(Sink(state.clone())),
        Err(McpSubmissionError::Invalid)
    ));
    assert_eq!(state.lock().unwrap().scalar, 0);
    assert_eq!(state.lock().unwrap().drops, 1);
    let fixture = Fixture::new();
    let state = Arc::new(Mutex::new(SinkState::default()));
    let mut direct = submission(&fixture, CancellationToken::new(), CancellationToken::new())
        .into_writer(Sink(state.clone()));
    assert_eq!(
        poll(&mut direct),
        Poll::Ready(Err(McpSubmissionError::Invalid))
    );
    assert_eq!(state.lock().unwrap().scalar, 0);
    assert!(!direct.was_attempted());
}

#[test]
fn changed_endpoint_headers_or_body_fail_before_scalar_or_vectored_sink() {
    for vector in [false, true] {
        for needle in [b"/mcp?".as_slice(), b"Bearer secret-auth", b"\"secret\":1"] {
            let fixture = Fixture::new();
            let state = Arc::new(Mutex::new(SinkState::default()));
            let mut driver = driver(&fixture, Sink(state.clone()));
            let original = driver.request_bytes().to_vec();
            let mut changed = original.clone();
            let index = changed
                .windows(needle.len())
                .position(|bytes| bytes == needle)
                .unwrap();
            changed[index] = b'X';
            let outcome = if vector {
                driver.poll_write_vectored(
                    &mut context(),
                    &[
                        IoSlice::new(&changed[..index]),
                        IoSlice::new(&changed[index..]),
                    ],
                )
            } else {
                driver.poll_write(&mut context(), &changed)
            };
            assert_eq!(outcome, Poll::Ready(Err(McpSubmissionError::Denied)));
            assert_eq!(
                driver.poll_write(&mut context(), &original),
                Poll::Ready(Err(McpSubmissionError::AlreadyAttempted))
            );
            assert!(!driver.was_attempted());
            let state = state.lock().unwrap();
            assert_eq!(state.scalar + state.vector, 0);
        }
    }
}

#[test]
fn partial_vectored_counts_pending_retries_and_intermediate_flush_keep_exact_cursor() {
    let fixture = Fixture::new();
    let state = Arc::new(Mutex::new(SinkState {
        writes: [Step::Accept(7), Step::Pending, Step::Accept(usize::MAX)].into(),
        ..SinkState::default()
    }));
    let mut driver = driver(&fixture, Sink(state.clone()));
    let request = driver.request_bytes().to_vec();
    assert_eq!(
        driver.poll_write_vectored(
            &mut context(),
            &[IoSlice::new(&request[..4]), IoSlice::new(&request[4..])]
        ),
        Poll::Ready(Ok(7))
    );
    assert_eq!(driver.acknowledged_bytes(), 7);
    assert_eq!(driver.poll_flush(&mut context()), Poll::Ready(Ok(())));
    assert!(!driver.is_complete());
    assert!(
        driver
            .poll_write_vectored(
                &mut context(),
                &[IoSlice::new(&request[7..10]), IoSlice::new(&request[10..])]
            )
            .is_pending()
    );
    assert_eq!(driver.acknowledged_bytes(), 7);
    assert_eq!(
        driver.poll_write(&mut context(), &request[7..]),
        Poll::Ready(Ok(request.len() - 7))
    );
    assert_eq!(driver.poll_flush(&mut context()), Poll::Ready(Ok(())));
    assert!(driver.is_complete());
    assert_eq!(state.lock().unwrap().bytes, request);
}

#[test]
fn replay_trailing_pipelined_output_and_repeated_completion_are_closed() {
    for mode in 0..3 {
        let fixture = Fixture::new();
        let state = Arc::new(Mutex::new(SinkState::default()));
        let mut driver = driver(&fixture, Sink(state.clone()));
        let request = driver.request_bytes().to_vec();
        let count = if mode == 0 { 7 } else { request.len() };
        assert_eq!(
            driver.poll_write(&mut context(), &request[..count]),
            Poll::Ready(Ok(count))
        );
        if mode == 2 {
            assert_eq!(driver.poll_flush(&mut context()), Poll::Ready(Ok(())));
        }
        assert!(matches!(
            driver.poll_write(&mut context(), &request),
            Poll::Ready(Err(_))
        ));
        assert!(matches!(
            driver.poll_flush(&mut context()),
            Poll::Ready(Err(_))
        ));
        assert_eq!(state.lock().unwrap().bytes, request[..count]);
        assert_eq!(state.lock().unwrap().scalar, 1);
    }
}

#[test]
fn empty_writes_are_inert_and_vector_metadata_is_bounded() {
    let fixture = Fixture::new();
    let state = Arc::new(Mutex::new(SinkState::default()));
    let mut driver = driver(&fixture, Sink(state.clone()));
    assert_eq!(driver.poll_write(&mut context(), &[]), Poll::Ready(Ok(0)));
    assert_eq!(
        driver.poll_write_vectored(&mut context(), &[IoSlice::new(&[])]),
        Poll::Ready(Ok(0))
    );
    assert!(!driver.was_attempted());
    let vectors: Vec<_> = (0..65).map(|_| IoSlice::new(&[])).collect();
    assert_eq!(
        driver.poll_write_vectored(&mut context(), &vectors),
        Poll::Ready(Err(McpSubmissionError::Limit))
    );
    assert_eq!(state.lock().unwrap().scalar, 0);
    assert_eq!(state.lock().unwrap().vector, 0);
}

#[test]
fn write_error_zero_invalid_count_and_panic_are_attempted_terminal_and_never_replayed() {
    for vector in [false, true] {
        for step in [Step::Fail, Step::Zero, Step::InvalidCount, Step::Panic] {
            let fixture = Fixture::new();
            let state = Arc::new(Mutex::new(SinkState {
                writes: [step].into(),
                ..SinkState::default()
            }));
            let mut driver = driver(&fixture, Sink(state.clone()));
            let request = driver.request_bytes().to_vec();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if vector {
                    driver.poll_write_vectored(&mut context(), &[IoSlice::new(&request)])
                } else {
                    driver.poll_write(&mut context(), &request)
                }
            }));
            if matches!(step, Step::Panic) {
                assert!(result.is_err());
            } else {
                assert_eq!(
                    result.unwrap(),
                    Poll::Ready(Err(McpSubmissionError::WriterFailed))
                );
            }
            assert!(driver.was_attempted());
            assert_eq!(driver.acknowledged_bytes(), 0);
            assert_eq!(
                driver.poll_write(&mut context(), &request),
                Poll::Ready(Err(McpSubmissionError::AlreadyAttempted))
            );
            if matches!(step, Step::Fail) {
                assert_eq!(state.lock().unwrap().bytes.len(), 1);
            }
        }
    }
}

#[test]
fn every_write_and_flush_stage_revalidates_after_queue_delay() {
    for stage in 0..5 {
        let fixture = Fixture::new();
        let state = Arc::new(Mutex::new(SinkState::default()));
        let mut driver = driver(&fixture, Sink(state.clone()));
        let request = driver.request_bytes().to_vec();
        if stage == 1 {
            assert_eq!(
                driver.poll_write(&mut context(), &request[..3]),
                Poll::Ready(Ok(3))
            );
        }
        if stage == 2 {
            state.lock().unwrap().writes.push_back(Step::Pending);
            assert!(
                driver
                    .poll_write_vectored(&mut context(), &[IoSlice::new(&request)])
                    .is_pending()
            );
        }
        if stage >= 3 {
            assert_eq!(
                driver.poll_write(&mut context(), &request),
                Poll::Ready(Ok(request.len()))
            );
        }
        if stage == 4 {
            state.lock().unwrap().flushes.push_back(Step::Pending);
            assert!(driver.poll_flush(&mut context()).is_pending());
        }
        let before = {
            let state = state.lock().unwrap();
            (state.scalar, state.vector, state.flush)
        };
        fixture.revoke();
        let outcome = if stage >= 3 {
            driver.poll_flush(&mut context()).map_ok(|()| 0)
        } else {
            let offset = driver.acknowledged_bytes();
            driver.poll_write_vectored(&mut context(), &[IoSlice::new(&request[offset..])])
        };
        assert_eq!(outcome, Poll::Ready(Err(McpSubmissionError::Denied)));
        let after = {
            let state = state.lock().unwrap();
            (state.scalar, state.vector, state.flush)
        };
        assert_eq!(before, after);
    }
}

#[test]
fn cancellation_sources_wake_pending_write_and_flush_with_independent_tokens() {
    for flush in [false, true] {
        for source in 0..6 {
            let mut fixture = Fixture::new();
            let state = Arc::new(Mutex::new(SinkState::default()));
            let preparation = CancellationToken::new();
            let execution = CancellationToken::new();
            let mut driver = submission(&fixture, preparation.clone(), execution.clone())
                .into_http_driver(Sink(state.clone()))
                .unwrap();
            let request = driver.request_bytes().to_vec();
            let counter = Arc::new(Counter(AtomicUsize::new(0)));
            let waker = Waker::from(counter.clone());
            let mut cx = Context::from_waker(&waker);
            if flush {
                assert_eq!(
                    driver.poll_write(&mut cx, &request),
                    Poll::Ready(Ok(request.len()))
                );
                state.lock().unwrap().flushes.push_back(Step::Pending);
                assert!(driver.poll_flush(&mut cx).is_pending());
            } else {
                state.lock().unwrap().writes.push_back(Step::Pending);
                assert!(
                    driver
                        .poll_write_vectored(&mut cx, &[IoSlice::new(&request)])
                        .is_pending()
                );
            }
            match source {
                0 => {
                    preparation.cancel();
                }
                1 => {
                    execution.cancel();
                }
                2 => {
                    assert!(fixture.turn_handle().cancel());
                }
                3 => {
                    fixture.close();
                }
                4 => {
                    fixture.runtime_owner.retire();
                }
                _ => {
                    fixture.runtime_owner.install(binding()).unwrap();
                }
            }
            assert!(counter.0.load(Ordering::SeqCst) > 0);
            let before = {
                let state = state.lock().unwrap();
                (state.scalar, state.vector, state.flush)
            };
            assert_eq!(
                driver.poll_flush(&mut cx),
                Poll::Ready(Err(McpSubmissionError::Cancelled))
            );
            assert_eq!(before, {
                let state = state.lock().unwrap();
                (state.scalar, state.vector, state.flush)
            });
        }
    }
}

#[test]
fn cancellation_during_delegation_wins_without_erasing_acknowledged_effects() {
    for flush in [false, true] {
        let fixture = Fixture::new();
        let state = Arc::new(Mutex::new(SinkState::default()));
        let cancellation = CancellationToken::new();
        let mut driver = submission(&fixture, CancellationToken::new(), cancellation.clone())
            .into_http_driver(Sink(state.clone()))
            .unwrap();
        let request = driver.request_bytes().to_vec();
        if flush {
            assert_eq!(
                driver.poll_write(&mut context(), &request),
                Poll::Ready(Ok(request.len()))
            );
        }
        state.lock().unwrap().callback = Some(Box::new(move || {
            cancellation.cancel();
        }));
        if flush {
            assert_eq!(
                driver.poll_flush(&mut context()),
                Poll::Ready(Err(McpSubmissionError::Cancelled))
            );
        } else {
            assert_eq!(
                driver.poll_write_vectored(&mut context(), &[IoSlice::new(&request)]),
                Poll::Ready(Err(McpSubmissionError::Cancelled))
            );
        }
        assert_eq!(driver.acknowledged_bytes(), request.len());
        assert!(!driver.is_complete());
        assert!(driver.was_attempted());
        assert_eq!(state.lock().unwrap().bytes, request);
    }
}

#[test]
fn dropping_unpolled_pending_or_partial_driver_releases_sink_without_replay() {
    for stage in 0..3 {
        let fixture = Fixture::new();
        let state = Arc::new(Mutex::new(SinkState::default()));
        let mut driver = driver(&fixture, Sink(state.clone()));
        let request = driver.request_bytes().to_vec();
        if stage == 1 {
            state.lock().unwrap().writes.push_back(Step::Pending);
            assert!(driver.poll_write(&mut context(), &request).is_pending());
        }
        if stage == 2 {
            assert_eq!(
                driver.poll_write(&mut context(), &request[..3]),
                Poll::Ready(Ok(3))
            );
        }
        drop(driver);
        assert_eq!(state.lock().unwrap().drops, 1);
        assert!(block_on(fixture.claim("call", CancellationToken::new())).is_err());
    }
}

#[test]
fn retirement_wakers_reenter_without_holding_registry_mutex() {
    use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
    let fixture = Fixture::new();
    let state = Arc::new(Mutex::new(SinkState {
        writes: [Step::Pending].into(),
        ..SinkState::default()
    }));
    let mut driver = driver(&fixture, Sink(state));
    let request = driver.request_bytes().to_vec();
    let registry = fixture.registry.clone();
    let (waker, observed) = reentrant_waker(Callback::Wake, move || {
        assert!(registry.state.try_lock().is_ok());
    });
    assert!(
        driver
            .poll_write(&mut Context::from_waker(&waker), &request)
            .is_pending()
    );
    fixture.runtime_owner.retire();
    assert!(observed.calls() > 0);
    assert_eq!(
        driver.poll_flush(&mut context()),
        Poll::Ready(Err(McpSubmissionError::Cancelled))
    );
}
