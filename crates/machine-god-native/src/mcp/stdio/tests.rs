use super::*;

#[test]
fn required_readiness_observation_tracks_connection_failure_and_drop_without_retention() {
    let connection = McpStdioConnection::inert_for_test();
    let observation = connection.readiness();
    assert!(observation.is_ready());
    connection.shared.finish(McpStdioError::Closed);
    assert!(!observation.is_ready());
    let connection = McpStdioConnection::inert_for_test();
    let observation = connection.readiness();
    assert!(observation.is_ready());
    connection.close();
    assert!(!observation.is_ready());
    drop(connection);
    assert!(observation.0.upgrade().is_none());
}
mod runtime;

#[test]
fn runtime_set_admits_pinned_tool_count_and_rejects_overflow_atomically() {
    let scope = NativeOwnedWorkerScope::new();
    let shared = Arc::new(Shared::new(WireLimits::default(), CancellationToken::new()));
    let connection = McpStdioConnection {
        shared: shared.clone(),
        completion: scope.completion(),
    };
    let fixture = super::super::submission::tests::Fixture::new();
    connection
        .admit_runtimes(vec![fixture.runtime.clone(); 2048])
        .unwrap();
    assert_eq!(shared.runtimes.lock().unwrap().len(), 2048);
    assert_eq!(
        connection.admit_runtimes(vec![fixture.runtime.clone(); 2049]),
        Err(McpStdioError::Capacity)
    );
    let retained = shared.runtimes.lock().unwrap();
    assert_eq!(retained.len(), 2048);
    assert!(
        retained
            .iter()
            .all(|runtime| Arc::ptr_eq(runtime, &fixture.runtime))
    );
    drop(retained);
    scope.close();
}

#[test]
fn received_frame_preserves_original_catalog_json_without_roundtrip() {
    let scope = NativeOwnedWorkerScope::new();
    let shared = Arc::new(Shared::new(WireLimits::default(), CancellationToken::new()));
    let original = br#"{ "jsonrpc":"2.0", "id":7, "result":{"tools":[{"name":"private-tool","inputSchema":{"const":9007199254740993.0}}]} }"#;
    shared
        .state
        .lock()
        .unwrap()
        .frames
        .push_back(original.to_vec());
    let connection = McpStdioConnection {
        shared: shared.clone(),
        completion: scope.completion(),
    };
    drop(connection.receive_frame());
    assert_eq!(shared.state.lock().unwrap().frames.len(), 1);
    let frame = futures_executor::block_on(connection.receive_frame()).unwrap();
    assert_eq!(frame.bytes(), original);
    assert_eq!(
        frame.envelope().id(),
        Some(&crate::mcp::protocol::RpcId::Integer(7))
    );
    assert!(!format!("{frame:?}").contains("private-tool"));
    let envelope = frame.into_envelope();
    assert_eq!(
        envelope.id(),
        Some(&crate::mcp::protocol::RpcId::Integer(7))
    );
    scope.close();
}

#[test]
fn control_lane_rejects_application_calls_and_frame_smuggling() {
    for bytes in [
        br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"x","arguments":{}}}"#
            .as_slice(),
        br#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"secret"}}"#,
        br#"{"jsonrpc":"2.0","id":1,"method":"prompts/get","params":{"name":"x"}}"#,
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n",
    ] {
        assert!(McpStdioControl::discovery(bytes).is_err());
    }
}

#[test]
fn modern_discovery_and_catalog_controls_reject_old_startup_methods() {
    for method in [
        "server/discover",
        "tools/list",
        "resources/list",
        "resources/templates/list",
        "prompts/list",
    ] {
        let bytes = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{method}"}}"#);
        let frame = McpStdioControl::discovery(bytes.as_bytes()).unwrap();
        assert_eq!(frame.bytes.last(), Some(&b'\n'));
        #[cfg(feature = "mcp-http")]
        assert_eq!(frame.json_bytes(), bytes.as_bytes());
    }
    let bytes = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    assert!(McpStdioControl::discovery(bytes).is_err());
    for method in [
        "initialize",
        "notifications/initialized",
        "notifications/cancelled",
    ] {
        let bytes = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{method}"}}"#);
        assert!(McpStdioControl::discovery(bytes.as_bytes()).is_err());
    }
    assert!(McpStdioControl::unsupported(&super::super::protocol::RpcId::Null).is_err());
}

#[test]
fn response_completion_is_once_even_after_consumption() {
    let response = Response::new();
    response.complete(Ok(7));
    response.complete(Ok(8));
    assert_eq!(futures_executor::block_on(response.wait()), Ok(7));
    response.complete(Err(McpStdioError::Process));
    assert!(response.value.lock().unwrap().is_none());
}

#[test]
fn unpolled_and_expired_control_futures_admit_nothing() {
    let scope = NativeOwnedWorkerScope::new();
    let shared = Arc::new(Shared::new(WireLimits::default(), CancellationToken::new()));
    let connection = McpStdioConnection {
        shared: shared.clone(),
        completion: scope.completion(),
    };
    let bytes = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    drop(connection.control(
        McpStdioControl::discovery(bytes).unwrap(),
        Instant::now() + std::time::Duration::from_secs(1),
    ));
    assert_eq!(shared.state.lock().unwrap().admitted, 0);
    assert_eq!(
        futures_executor::block_on(
            connection.control(McpStdioControl::discovery(bytes).unwrap(), Instant::now())
        ),
        Err(McpStdioError::Deadline)
    );
    assert_eq!(shared.state.lock().unwrap().admitted, 0);
    connection.close();
    assert!(!scope.completion().is_complete());
}

#[test]
fn queue_is_bounded_and_abandonment_remains_cancellation() {
    let scope = NativeOwnedWorkerScope::new();
    let shared = Arc::new(Shared::new(WireLimits::default(), CancellationToken::new()));
    let connection = McpStdioConnection {
        shared: shared.clone(),
        completion: scope.completion(),
    };
    let mut futures = Vec::new();
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    for _ in 0..MAX_MCP_STDIO_WRITES {
        let control =
            McpStdioControl::discovery(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
                .unwrap();
        let mut future =
            connection.control(control, Instant::now() + std::time::Duration::from_secs(1));
        assert!(future.as_mut().poll(&mut cx).is_pending());
        futures.push(future);
    }
    let control =
        McpStdioControl::discovery(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
    assert_eq!(
        futures_executor::block_on(
            connection.control(control, Instant::now() + std::time::Duration::from_secs(1))
        ),
        Err(McpStdioError::Capacity)
    );
    drop(futures);
    assert!(
        shared
            .state
            .lock()
            .unwrap()
            .queue
            .iter()
            .all(|queued| queued.cancel.is_cancelled())
    );
    shared.finish(McpStdioError::Closed);
    assert_eq!(shared.state.lock().unwrap().admitted, 0);
}

#[test]
fn debug_and_errors_omit_contents() {
    let control = McpStdioControl::discovery(
        br#"{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"secret":"credential"}}"#,
    )
    .unwrap();
    assert!(!format!("{control:?}").contains("credential"));
    assert_eq!(
        McpStdioError::Process.to_string(),
        "MCP stdio operation failed"
    );
}

#[test]
fn foreign_equal_generation_runtime_cannot_enter_queue() {
    let first = crate::mcp::submission::tests::Fixture::new();
    let foreign = crate::mcp::submission::tests::Fixture::new();
    first.ready("call");
    let submission =
        futures_executor::block_on(first.claim("call", CancellationToken::new())).unwrap();
    let scope = NativeOwnedWorkerScope::new();
    let shared = Arc::new(Shared::new(WireLimits::default(), CancellationToken::new()));
    let connection = McpStdioConnection {
        shared: shared.clone(),
        completion: scope.completion(),
    };
    assert_eq!(first.runtime.generation(), foreign.runtime.generation());
    connection
        .admit_runtimes(vec![foreign.runtime.clone()])
        .unwrap();
    assert_eq!(
        futures_executor::block_on(connection.submit(
            submission,
            Instant::now() + std::time::Duration::from_secs(1)
        )),
        Err(McpStdioError::ForeignRuntime)
    );
    assert_eq!(shared.state.lock().unwrap().admitted, 0);
}

#[test]
fn closure_wakes_pending_receiver_and_malformed_json_closes_admission() {
    let scope = NativeOwnedWorkerScope::new();
    let shared = Arc::new(Shared::new(WireLimits::default(), CancellationToken::new()));
    let connection = McpStdioConnection {
        shared: shared.clone(),
        completion: scope.completion(),
    };
    let mut first = connection.receive();
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(first.as_mut().poll(&mut cx).is_pending());
    assert!(matches!(
        connection.receive().as_mut().poll(&mut cx),
        Poll::Ready(Err(McpStdioError::Capacity))
    ));
    shared.finish(McpStdioError::Closed);
    assert!(matches!(
        first.as_mut().poll(&mut cx),
        Poll::Ready(Err(McpStdioError::Closed))
    ));
    drop(first);
    let mut state = shared.state.lock().unwrap();
    state.closed = None;
    state.frames.push_back(b"not-json".to_vec());
    drop(state);
    assert!(matches!(
        futures_executor::block_on(connection.receive()),
        Err(McpStdioError::Protocol)
    ));
    assert_eq!(shared.check(), Err(McpStdioError::Cancelled));
}
