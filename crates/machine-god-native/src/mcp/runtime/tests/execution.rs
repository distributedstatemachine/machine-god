use super::*;
use crate::mcp::execution::NativeMcpArchivedToolExecutor;
use futures_executor::block_on;
use futures_util::StreamExt;
use std::{os::unix::fs::DirBuilderExt, path::PathBuf};

struct Archive {
    path: PathBuf,
    storage: Arc<ToolResultArchive>,
    executor: Arc<NativeMcpArchivedToolExecutor>,
}
impl Archive {
    fn new() -> Self {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let path = std::env::temp_dir().join(format!("machine-god-mcp-execution-{nonce:x?}"));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let archive = Arc::new(ToolResultArchive::from_root_descriptor(
            rustix::fs::open(
                &path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .unwrap(),
        ));
        let executor = Arc::new(
            NativeMcpArchivedToolExecutor::new(Arc::new(NativeToolResultArchiveAdapter::new(
                archive.clone(),
            )))
            .unwrap(),
        );
        Self {
            path,
            storage: archive,
            executor,
        }
    }
    fn fixture(&self, response: &str, arguments: &[Value], batch: bool) -> Fixture {
        let response = response.to_owned();
        Fixture::with_executor(
            arguments,
            PermissionMode::Auto,
            self.executor.clone(),
            self.executor.execution_policy(),
            batch,
            move |runtime, writes| {
                scripted_candidate(
                    runtime,
                    writes,
                    &json!({"name":"lookup","inputSchema":{"type":"object"}}),
                    move |id| {
                        format!(r#"{{"jsonrpc":"2.0","id":{id},{response}}}"#)
                            .into_bytes()
                            .into()
                    },
                )
            },
        )
    }
    fn names(&self) -> usize {
        std::fs::read_dir(&self.path).unwrap().count()
    }
    fn read(&self, reference: &Value) -> ToolOutput {
        let reference: ArchivedToolResult =
            serde_json::from_value(reference["archive"].clone()).unwrap();
        let mut source = String::new();
        loop {
            let page = self
                .storage
                .read(
                    &reference.source_context,
                    &reference.handle,
                    source.len() + 1,
                    TOOL_RESULT_ARCHIVE_MAX_PAGE_BYTES,
                )
                .unwrap();
            source.push_str(&page.text);
            if page.end_byte == page.source_total_bytes {
                break;
            }
        }
        serde_json::from_str(&source).unwrap()
    }
}
impl Drop for Archive {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}

fn scripted_candidate(
    runtime: &NativeMcpRuntime,
    writes: Arc<Mutex<Vec<u8>>>,
    tool: &Value,
    response: impl Fn(i64) -> Box<[u8]> + Send + Sync + 'static,
) -> NativeMcpRuntimeCandidate {
    let mut builder = McpCatalogBuilder::new(
        McpCatalogKind::Tools,
        ProtocolVersion::Modern,
        McpCatalogLimits::default(),
    )
    .unwrap();
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"resultType":"complete","tools":[{}]}}}}"#,
        serde_json::to_string(tool).unwrap()
    );
    builder
        .append_response(body.as_bytes(), &RpcId::Integer(1), None, 0)
        .unwrap();
    let catalog =
        McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default())
            .unwrap();
    runtime
        .prepare_candidate(
            vec![NativeMcpServerCandidate {
                server: Arc::from("calendar"),
                configuration: Arc::from(&b"configuration"[..]),
                authentication: Arc::from(&b"credential"[..]),
                catalogs: vec![catalog],
                peer: NativeMcpOwnedPeer::Script(
                    script::ScriptPeer::new(writes).with_response(response),
                ),
            }],
            &[MCP_SELECT_TOOL_NAME],
        )
        .unwrap()
}
fn result(events: &[EngineEvent]) -> &ToolOutput {
    events
        .iter()
        .find_map(|event| match &event.payload {
            TurnEvent::ToolFinished { call_id, output } if call_id.as_str() == "call-0" => {
                Some(output)
            }
            _ => None,
        })
        .unwrap()
}
fn record(fixture: &Fixture) -> SessionRecord {
    fixture
        .store
        .record(&SessionId::new("runtime").unwrap())
        .unwrap()
}
fn persisted(record: &SessionRecord, id: &str) -> ToolOutput {
    record
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|block| match block {
            ContentBlock::ToolResult { call_id, output } if call_id.as_str() == id => {
                Some(output.clone())
            }
            _ => None,
        })
        .unwrap()
}
fn assert_calls(fixture: &Fixture, count: usize) {
    assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), count);
    let writes = fixture.writes.lock().unwrap();
    assert_eq!(
        writes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .count(),
        count
    );
}

#[test]
fn complete_and_tool_failure_are_exact_inline_without_archive_io() {
    for is_error in [false, true] {
        let archive = Archive::new();
        let raw = format!(
            r#"{{"resultType":"complete","content":[{{"type":"text","text":"done"}}],"isError":{is_error}}}"#
        );
        let fixture = archive.fixture(&format!(r#""result":{raw}"#), &[json!({})], false);
        let events = fixture.run();
        let expected = ToolOutput {
            content: machine_god_core::json::from_str(&raw).unwrap(),
            is_error,
        };
        assert_eq!(result(&events), &expected);
        assert_eq!(persisted(&record(&fixture), "call-0"), expected);
        assert_eq!(archive.names(), 0);
        assert_calls(&fixture, 1);
        assert_eq!(fixture.provider.requests().len(), 3);
        let wire = machine_god_core::json::from_slice(&fixture.writes.lock().unwrap()).unwrap();
        let serialized = serde_json::to_string(&wire).unwrap();
        assert!(!serialized.contains("elicitation"));
    }
}

#[test]
fn large_result_preserves_exact_numbers_private_keys_and_real_archive_read() {
    let archive = Archive::new();
    let raw = format!(
        r#"{{"resultType":"complete","content":[{{"type":"text","text":"{}"}}],"structuredContent":{{"n":9007199254740993.0000001,"tiny":1e-99999,"zero":-0,"$serde_json::private::Number":{{"$serde_json::private::RawValue":"literal"}}}}}}"#,
        "x".repeat(70_000)
    );
    let fixture = archive.fixture(&format!(r#""result":{raw}"#), &[json!({})], false);
    let events = fixture.run();
    let expected = ToolOutput::success(machine_god_core::json::from_str(&raw).unwrap());
    assert_eq!(result(&events), &expected);
    let durable = persisted(&record(&fixture), "call-0");
    assert_eq!(durable.content["type"], "tool_result_archive");
    assert_eq!(archive.read(&durable.content), expected);
    assert_calls(&fixture, 1);
    assert!(
        serde_json::to_string(&fixture.provider.requests().last().unwrap().request)
            .unwrap()
            .contains("tool_result_archive")
    );
}

#[test]
fn protocol_failure_is_an_explicit_error_not_completed_content_or_input() {
    let archive = Archive::new();
    let raw = r#"{"code":-32602,"message":"invalid call","data":{"exact":1e-99999,"$serde_json::private::Number":"literal"}}"#;
    let fixture = archive.fixture(&format!(r#""error":{raw}"#), &[json!({})], false);
    let events = fixture.run();
    let output = result(&events);
    assert!(output.is_error);
    assert_eq!(output.content["resultType"], "protocol_failure");
    assert_eq!(
        output.content["error"],
        machine_god_core::json::from_str(raw).unwrap()
    );
    assert_eq!(fixture.provider.requests().len(), 3);
    assert_calls(&fixture, 1);
}

#[test]
fn unresolved_input_stops_real_turn_with_unknown_unexecuted_sibling() {
    let archive = Archive::new();
    let raw = r#""result":{"resultType":"input_required","inputRequests":{"confirm":{"method":"elicitation/create","params":{"message":"Continue?","requestedSchema":{"type":"object","properties":{"confirmed":{"type":"boolean"}}}}}},"requestState":{"exact":1e-99999,"zero":-0}}"#;
    let fixture = archive.fixture(raw, &[json!({}), json!({})], true);
    let events = fixture.run();
    assert!(result(&events).is_error);
    assert_eq!(result(&events).content["resultType"], "input_required");
    assert_eq!(
        serde_json::to_string(&result(&events).content["requestState"]["exact"]).unwrap(),
        "1e-99999"
    );
    assert_eq!(fixture.provider.requests().len(), 2);
    assert_calls(&fixture, 1);
    assert!(matches!(
        events.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        }
    ));
    assert_eq!(
        persisted(&record(&fixture), "call-1").content["code"],
        "tool_result_unknown"
    );
}

#[test]
fn malformed_result_and_output_schema_failure_never_archive_success() {
    for (schema, response) in [
        (None, r#""result":{"resultType":"input_required"}"#),
        (None, r#""result":{"content":[{"type":"text"}]}"#),
        (
            Some(
                json!({"type":"object","required":["count"],"properties":{"count":{"type":"integer"}}}),
            ),
            r#""result":{"content":[],"structuredContent":{"count":"wrong"}}"#,
        ),
    ] {
        let archive = Archive::new();
        let mut descriptor = json!({"name":"lookup","inputSchema":{"type":"object"}});
        if let Some(schema) = schema {
            descriptor["outputSchema"] = schema;
        }
        let fixture = Fixture::with_executor(
            &[json!({})],
            PermissionMode::Auto,
            archive.executor.clone(),
            archive.executor.execution_policy(),
            false,
            move |runtime, writes| {
                scripted_candidate(runtime, writes, &descriptor, move |id| {
                    format!(r#"{{"jsonrpc":"2.0","id":{id},{response}}}"#)
                        .into_bytes()
                        .into()
                })
            },
        );
        let events = fixture.run();
        assert!(result(&events).is_error);
        assert!(result(&events).content.get("resultType").is_none());
        assert_eq!(archive.names(), 0);
        assert_calls(&fixture, 1);
    }
}

#[test]
fn input_archive_is_inert_until_polled_and_preserves_original_arguments() {
    let archive = Archive::new();
    let context = ToolContext {
        session_id: SessionId::new("runtime").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("life").unwrap(),
        turn_id: TurnId::new("turn-1").unwrap(),
        call_id: ToolCallId::new("call-0").unwrap(),
    };
    let name = ToolName::new("lookup").unwrap();
    let arguments = json!({"text":"x".repeat(65_520)});
    drop(archive.executor.persist_arguments(
        context.clone(),
        &name,
        &arguments,
        CancellationToken::new(),
    ));
    assert_eq!(archive.names(), 0);
    let reference = block_on(archive.executor.persist_arguments(
        context,
        &name,
        &arguments,
        CancellationToken::new(),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(reference["type"], "tool_arguments_archive");
    assert_eq!(archive.read(&reference).content, arguments);
}

#[test]
fn cancellation_before_execution_never_writes_or_archives() {
    let archive = Archive::new();
    let fixture = archive.fixture(r#""result":{"content":[]}"#, &[json!({})], false);
    fixture.conversation.enqueue("go".into()).unwrap();
    let mut turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    loop {
        let event = block_on(turn.next()).unwrap().unwrap();
        if matches!(event.payload, TurnEvent::ToolStarted { call, .. } if call.id.as_str() == "call-0")
        {
            break;
        }
    }
    assert!(turn.handle().unwrap().cancel());
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(matches!(
        &events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert!(fixture.writes.lock().unwrap().is_empty());
    assert_eq!(archive.names(), 0);
    assert_eq!(
        persisted(&record(&fixture), "call-0").content["code"],
        "tool_result_unknown"
    );
}

struct CancelAfterReceipt {
    inner: Arc<NativeMcpArchivedToolExecutor>,
    handle: Arc<Mutex<Option<TurnHandle>>>,
}
impl NativeMcpToolExecutor for CancelAfterReceipt {
    fn execute(
        &self,
        call: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, std::result::Result<ToolExecution, ToolError>> {
        Box::pin(async move {
            let execution = self.inner.execute(call).await?;
            assert!(self.handle.lock().unwrap().as_ref().unwrap().cancel());
            Ok(execution)
        })
    }
    fn persist_arguments<'a>(
        &'a self,
        context: ToolContext,
        tool: &'a ToolName,
        arguments: &'a Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, std::result::Result<Option<Value>, ToolError>> {
        self.inner
            .persist_arguments(context, tool, arguments, cancellation)
    }
}

#[test]
fn cancellation_after_real_publication_keeps_archive_receipt_and_exact_event() {
    let archive = Archive::new();
    let handle = Arc::new(Mutex::new(None));
    let executor = Arc::new(CancelAfterReceipt {
        inner: archive.executor.clone(),
        handle: handle.clone(),
    });
    let raw = format!(
        r#"{{"content":[{{"type":"text","text":"{}"}}]}}"#,
        "x".repeat(70_000)
    );
    let expected = ToolOutput::success(machine_god_core::json::from_str(&raw).unwrap());
    let fixture = Fixture::with_executor(
        &[json!({}), json!({})],
        PermissionMode::Auto,
        executor,
        archive.executor.execution_policy(),
        true,
        move |runtime, writes| {
            scripted_candidate(
                runtime,
                writes,
                &json!({"name":"lookup","inputSchema":{"type":"object"}}),
                move |id| {
                    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{raw}}}"#)
                        .into_bytes()
                        .into()
                },
            )
        },
    );
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    *handle.lock().unwrap() = turn.handle();
    let events: Vec<_> = block_on(turn.collect::<Vec<_>>())
        .into_iter()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(result(&events), &expected);
    assert!(matches!(
        events.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    let stored = persisted(&record(&fixture), "call-0");
    assert_eq!(stored.content["type"], "tool_result_archive");
    assert_eq!(archive.read(&stored.content), expected);
    assert_eq!(
        persisted(&record(&fixture), "call-1").content["code"],
        "tool_result_unknown"
    );
    assert_calls(&fixture, 1);
    assert_eq!(fixture.provider.requests().len(), 2);
}

#[test]
fn cancellation_after_wire_before_admission_does_not_start_archive() {
    let archive = Archive::new();
    let handle: Arc<Mutex<Option<TurnHandle>>> = Arc::default();
    let captured = handle.clone();
    let fixture = Fixture::with_executor(
        &[json!({})],
        PermissionMode::Auto,
        archive.executor.clone(),
        archive.executor.execution_policy(),
        false,
        move |runtime, writes| {
            scripted_candidate(
                runtime,
                writes,
                &json!({"name":"lookup","inputSchema":{"type":"object"}}),
                move |id| {
                    assert!(captured.lock().unwrap().as_ref().unwrap().cancel());
                    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"content":[{{"type":"text","text":"{}"}}]}}}}"#, "x".repeat(70_000)).into_bytes().into()
                },
            )
        },
    );
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    *handle.lock().unwrap() = turn.handle();
    let events: Vec<_> = block_on(turn.collect::<Vec<_>>())
        .into_iter()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert!(matches!(
        events.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert_eq!(archive.names(), 0);
    assert_calls(&fixture, 1);
    assert!(persisted(&record(&fixture), "call-0").is_error);
}

#[test]
fn large_original_arguments_are_archived_without_changing_actual_wire() {
    let archive = Archive::new();
    let arguments = json!({"text":"x".repeat(65_520)});
    let fixture = Fixture::with_executor(
        std::slice::from_ref(&arguments),
        PermissionMode::Yolo,
        archive.executor.clone(),
        archive.executor.execution_policy(),
        false,
        |runtime, writes| {
            scripted_candidate(
                runtime,
                writes,
                &json!({"name":"lookup","inputSchema":{"type":"object"}}),
                |id| {
                    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"content":[]}}}}"#)
                        .into_bytes()
                        .into()
                },
            )
        },
    );
    fixture.run();
    let wire = machine_god_core::json::from_slice(&fixture.writes.lock().unwrap()).unwrap();
    assert_eq!(wire["params"]["arguments"], arguments);
    let record = record(&fixture);
    let stored = record
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|block| match block {
            ContentBlock::ToolCall { call } if call.id.as_str() == "call-0" => {
                Some(&call.arguments)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(stored["type"], "tool_arguments_archive");
    assert_eq!(archive.read(stored).content, arguments);
    assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 0);
}
