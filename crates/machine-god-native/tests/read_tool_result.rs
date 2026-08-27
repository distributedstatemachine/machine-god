use futures_executor::block_on;
use futures_util::task::noop_waker;
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, Message, PreparedToolAuthorization, Role,
    SessionId, SessionIncarnationId, SessionRecord, SessionRevision, SessionStore,
    SessionStoreError, SessionStoreErrorKind, Tool, ToolCall, ToolCallId, ToolContext, ToolError,
    ToolErrorKind, ToolName, ToolOutput, TurnId,
};
use machine_god_native::{
    READ_TOOL_RESULT_TOOL_NAME, ReadToolResultConfigErrorKind, ReadToolResultLimits,
    ReadToolResultTool,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

const HANDLE_DOMAIN: &[u8] = b"machine-god/tool-result-handle/v1\0";
const MAX_SCAN_BYTES: usize = 8 * 1024 * 1024;

type LoadResult = Result<Option<SessionRecord>, SessionStoreError>;

enum LoadStep {
    Ready(LoadResult),
    Pending,
    CancelThenReady {
        cancellation: CancellationToken,
        result: LoadResult,
    },
}

#[derive(Default)]
struct StoreProbe {
    loads: AtomicUsize,
    polls: AtomicUsize,
    drops: AtomicUsize,
    live: AtomicUsize,
}

#[derive(Clone)]
struct ScriptStore {
    steps: Arc<Mutex<VecDeque<LoadStep>>>,
    probe: Arc<StoreProbe>,
}

impl ScriptStore {
    fn new(steps: impl IntoIterator<Item = LoadStep>) -> Self {
        Self {
            steps: Arc::new(Mutex::new(steps.into_iter().collect())),
            probe: Arc::new(StoreProbe::default()),
        }
    }

    fn reader(&self) -> ReadToolResultTool {
        ReadToolResultTool::shared_session_store(Arc::new(self.clone()))
    }

    fn bounded_reader(&self, limits: ReadToolResultLimits) -> ReadToolResultTool {
        ReadToolResultTool::with_limits(Arc::new(self.clone()), limits).unwrap()
    }
}

impl SessionStore for ScriptStore {
    fn load(&self, _id: SessionId) -> BoxFuture<'_, LoadResult> {
        self.probe.loads.fetch_add(1, Ordering::SeqCst);
        let step = self
            .steps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
            .expect("unexpected session load");
        self.probe.live.fetch_add(1, Ordering::SeqCst);
        Box::pin(LoadFuture {
            state: LoadState::from(step),
            probe: Arc::clone(&self.probe),
        })
    }

    fn save(
        &self,
        _record: SessionRecord,
        _expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        Box::pin(async { panic!("read_tool_result must not save sessions") })
    }
}

enum LoadState {
    Ready(Option<LoadResult>),
    Pending,
    CancelThenReady {
        cancellation: CancellationToken,
        result: Option<LoadResult>,
    },
}

impl From<LoadStep> for LoadState {
    fn from(step: LoadStep) -> Self {
        match step {
            LoadStep::Ready(result) => Self::Ready(Some(result)),
            LoadStep::Pending => Self::Pending,
            LoadStep::CancelThenReady {
                cancellation,
                result,
            } => Self::CancelThenReady {
                cancellation,
                result: Some(result),
            },
        }
    }
}

struct LoadFuture {
    state: LoadState,
    probe: Arc<StoreProbe>,
}

impl Future for LoadFuture {
    type Output = LoadResult;

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.probe.polls.fetch_add(1, Ordering::SeqCst);
        match &mut self.state {
            LoadState::Ready(result) => {
                Poll::Ready(result.take().expect("load polled after ready"))
            }
            LoadState::Pending => Poll::Pending,
            LoadState::CancelThenReady {
                cancellation,
                result,
            } => {
                cancellation.cancel();
                Poll::Ready(result.take().expect("load polled after ready"))
            }
        }
    }
}

impl Drop for LoadFuture {
    fn drop(&mut self) {
        let previous = self.probe.live.fetch_sub(1, Ordering::SeqCst);
        assert!(previous > 0, "load-future live count underflowed");
        self.probe.drops.fetch_add(1, Ordering::SeqCst);
    }
}

struct Fixture {
    session_id: SessionId,
    incarnation_id: SessionIncarnationId,
    call_id: ToolCallId,
    output: ToolOutput,
    serialized: Vec<u8>,
    handle: String,
    record: SessionRecord,
}

impl Fixture {
    fn new(session: &str, incarnation: &str, call: &str, output: ToolOutput) -> Self {
        let session_id = SessionId::new(session).unwrap();
        let incarnation_id = SessionIncarnationId::new(incarnation).unwrap();
        let call_id = ToolCallId::new(call).unwrap();
        let serialized = serde_json::to_vec(&output).unwrap();
        let handle = handle_for(&session_id, &incarnation_id, &call_id, &serialized);
        let record = record_with_results(
            session_id.clone(),
            incarnation_id.clone(),
            [(call_id.clone(), output.clone())],
        );
        Self {
            session_id,
            incarnation_id,
            call_id,
            output,
            serialized,
            handle,
            record,
        }
    }

    fn context(&self) -> ToolContext {
        context(&self.session_id, &self.incarnation_id)
    }
}

fn record_with_results(
    session_id: SessionId,
    incarnation_id: SessionIncarnationId,
    results: impl IntoIterator<Item = (ToolCallId, ToolOutput)>,
) -> SessionRecord {
    let mut record = SessionRecord::empty(session_id, incarnation_id);
    record.revision = SessionRevision(1);
    record.messages.push(Message {
        role: Role::Tool,
        content: results
            .into_iter()
            .map(|(call_id, output)| ContentBlock::ToolResult { call_id, output })
            .collect(),
    });
    record
}

fn context(session_id: &SessionId, incarnation_id: &SessionIncarnationId) -> ToolContext {
    ToolContext {
        session_id: session_id.clone(),
        session_incarnation_id: incarnation_id.clone(),
        turn_id: TurnId::new("reader-turn").unwrap(),
        call_id: ToolCallId::new("reader-call").unwrap(),
    }
}

fn call(arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("reader-call").unwrap(),
        name: ToolName::new(READ_TOOL_RESULT_TOOL_NAME).unwrap(),
        arguments,
    }
}

fn arguments(handle: &str, start_byte: usize, byte_count: usize) -> Value {
    json!({
        "handle": handle,
        "start_byte": start_byte,
        "byte_count": byte_count
    })
}

fn synthetic_handle(digit: char) -> String {
    format!("tool-result-sha256-{}", digit.to_string().repeat(64))
}

fn handle_for(
    session_id: &SessionId,
    incarnation_id: &SessionIncarnationId,
    call_id: &ToolCallId,
    serialized: &[u8],
) -> String {
    let mut digest = Sha256::new();
    digest.update(HANDLE_DOMAIN);
    for component in [
        session_id.as_str().as_bytes(),
        incarnation_id.as_str().as_bytes(),
        call_id.as_str().as_bytes(),
        serialized,
    ] {
        digest.update(u64::try_from(component.len()).unwrap().to_be_bytes());
        digest.update(component);
    }
    format!("tool-result-sha256-{:x}", digest.finalize())
}

fn execute(
    tool: &ReadToolResultTool,
    context: ToolContext,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<ToolOutput, ToolError> {
    block_on(tool.execute(context, arguments, cancellation))
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = noop_waker();
    future.poll(&mut Context::from_waker(&waker))
}

fn assert_error(
    error: &ToolError,
    kind: ToolErrorKind,
    code: &str,
    message: &str,
    retryable: bool,
) {
    assert_eq!(error.kind, kind);
    assert_eq!(error.code, code);
    assert_eq!(error.message, message);
    assert_eq!(error.retryable, retryable);
    assert_eq!(error.to_string(), format!("{code}: {message}"));
}

fn assert_invalid(error: &ToolError) {
    assert_error(
        error,
        ToolErrorKind::InvalidInput,
        "read_tool_result_invalid_arguments",
        "read_tool_result arguments are invalid",
        false,
    );
}

fn assert_not_found(error: &ToolError) {
    assert_error(
        error,
        ToolErrorKind::Unavailable,
        "read_tool_result_not_found",
        "tool result is unavailable",
        false,
    );
}

fn page_text(output: &ToolOutput) -> &str {
    output.content["serialized_tool_output"]
        .as_str()
        .expect("page text is a string")
}

#[test]
fn schema_and_preparation_are_strict_bounded_and_canonical() {
    let store = ScriptStore::new([]);
    let tool = store.reader();
    let handle = synthetic_handle('a');
    assert_eq!(
        tool.spec().input_schema,
        json!({
            "type": "object",
            "properties": {
                "handle": {
                    "type": "string",
                    "minLength": 83,
                    "maxLength": 83,
                    "pattern": "^tool-result-sha256-[0-9a-f]{64}$"
                },
                "start_byte": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 65_537,
                    "default": 1
                },
                "byte_count": {
                    "type": "integer",
                    "minimum": 4,
                    "maximum": 16_384,
                    "default": 8_192
                }
            },
            "required": ["handle"],
            "additionalProperties": false
        })
    );

    let prepared = tool.prepare(call(json!({ "handle": handle }))).unwrap();
    assert_eq!(
        prepared.authorization(),
        &PreparedToolAuthorization::NoAuthorityRequired
    );
    assert_eq!(
        prepared.arguments(),
        &json!({
            "handle": synthetic_handle('a'),
            "start_byte": 1,
            "byte_count": 8_192
        })
    );
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 0);

    let invalid = [
        Value::Null,
        json!({}),
        json!({ "handle": synthetic_handle('A') }),
        json!({ "handle": synthetic_handle('a'), "unknown": true }),
        json!({ "handle": synthetic_handle('a'), "query": "secret" }),
        json!({ "handle": synthetic_handle('a'), "start_byte": 1.0 }),
        json!({ "handle": synthetic_handle('a'), "byte_count": 4.0 }),
    ];
    for value in invalid {
        assert_invalid(&tool.prepare(call(value)).unwrap_err());
    }
    let direct_context = context(
        &SessionId::new("direct-validation-session").unwrap(),
        &SessionIncarnationId::new("direct-validation-incarnation").unwrap(),
    );
    assert_invalid(
        &execute(
            &tool,
            direct_context,
            json!({ "handle": synthetic_handle('a'), "query": "forbidden" }),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 0);

    let oversized = json!({
        "handle": synthetic_handle('a'),
        "query": "x".repeat(512)
    });
    assert!(serde_json::to_vec(&oversized).unwrap().len() > 512);
    assert_error(
        &tool.prepare(call(oversized)).unwrap_err(),
        ToolErrorKind::InvalidInput,
        "read_tool_result_resource_limit",
        "read_tool_result resource limit exceeded",
        false,
    );

    let mut wrong_name = call(json!({ "handle": synthetic_handle('a') }));
    wrong_name.name = ToolName::new("other_tool").unwrap();
    assert_invalid(&tool.prepare(wrong_name).unwrap_err());
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 0);
}

#[test]
fn public_page_start_and_configuration_bounds_are_exact() {
    let defaults = ReadToolResultLimits::default();
    assert_eq!(defaults.max_active_reads(), 2);
    assert_eq!(defaults.max_scanned_tool_results(), 4_096);
    assert_eq!(defaults.max_serialized_scan_bytes(), MAX_SCAN_BYTES);
    let production_max = ReadToolResultLimits::new(8, 4_096, MAX_SCAN_BYTES).unwrap();
    assert_eq!(production_max.max_active_reads(), 8);
    assert_eq!(production_max.max_scanned_tool_results(), 4_096);
    assert_eq!(production_max.max_serialized_scan_bytes(), MAX_SCAN_BYTES);
    for invalid in [
        ReadToolResultLimits::new(0, 1, 1),
        ReadToolResultLimits::new(9, 1, 1),
        ReadToolResultLimits::new(1, 0, 1),
        ReadToolResultLimits::new(1, 4_097, 1),
        ReadToolResultLimits::new(1, 1, 0),
        ReadToolResultLimits::new(1, 1, MAX_SCAN_BYTES + 1),
    ] {
        let error = invalid.unwrap_err();
        assert_eq!(error.kind(), ReadToolResultConfigErrorKind::InvalidLimits);
        assert_eq!(error.to_string(), "invalid read_tool_result limits");
    }

    let empty_overhead = serde_json::to_vec(&ToolOutput::success("")).unwrap().len();
    let output = ToolOutput::success("x".repeat(65_536 - empty_overhead));
    let fixture = Fixture::new("boundary-session", "boundary-incarnation", "source", output);
    assert_eq!(fixture.serialized.len(), 65_536);
    let store = ScriptStore::new([
        LoadStep::Ready(Ok(Some(fixture.record.clone()))),
        LoadStep::Ready(Ok(Some(fixture.record.clone()))),
    ]);
    let tool = store.reader();

    for (start, count) in [(1, 4), (65_537, 16_384)] {
        tool.prepare(call(arguments(&fixture.handle, start, count)))
            .unwrap();
    }
    for (start, count) in [(0, 4), (65_538, 4), (1, 3), (1, 16_385)] {
        assert_invalid(
            &tool
                .prepare(call(arguments(&fixture.handle, start, count)))
                .unwrap_err(),
        );
    }

    let maximum_page = execute(
        &tool,
        fixture.context(),
        arguments(&fixture.handle, 1, 16_384),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(page_text(&maximum_page).len(), 16_384);
    assert_eq!(maximum_page.content["end_byte"], 16_384);
    assert_eq!(maximum_page.content["has_more"], true);

    let exact_eof = execute(
        &tool,
        fixture.context(),
        arguments(&fixture.handle, 65_537, 4),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(page_text(&exact_eof), "");
    assert_eq!(exact_eof.content["end_byte"], 65_536);
    assert_eq!(exact_eof.content["total_bytes"], 65_536);
    assert_eq!(exact_eof.content["has_more"], false);
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 2);
}

#[test]
fn stable_lookup_is_session_and_incarnation_scoped_with_fixed_not_found_collapse() {
    let fixture = Fixture::new(
        "scope-session-a",
        "scope-incarnation-a",
        "original-call",
        ToolOutput::success(json!({ "answer": "stable" })),
    );
    assert_eq!(
        fixture.handle,
        handle_for(
            &fixture.session_id,
            &fixture.incarnation_id,
            &fixture.call_id,
            &fixture.serialized
        )
    );
    assert_eq!(fixture.handle.len(), 83);

    let stable_store = ScriptStore::new([
        LoadStep::Ready(Ok(Some(fixture.record.clone()))),
        LoadStep::Ready(Ok(Some(fixture.record.clone()))),
    ]);
    let stable_tool = stable_store.reader();
    let first = execute(
        &stable_tool,
        fixture.context(),
        arguments(&fixture.handle, 1, 16_384),
        CancellationToken::new(),
    )
    .unwrap();
    let second = execute(
        &stable_tool,
        fixture.context(),
        arguments(&fixture.handle, 1, 16_384),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(first, second);
    assert_eq!(page_text(&first).as_bytes(), fixture.serialized);
    assert_eq!(stable_store.probe.loads.load(Ordering::SeqCst), 2);

    let session_b = SessionId::new("scope-session-b").unwrap();
    let cross_record = record_with_results(
        session_b.clone(),
        fixture.incarnation_id.clone(),
        [(fixture.call_id.clone(), fixture.output.clone())],
    );
    let wrong_incarnation = SessionIncarnationId::new("scope-incarnation-reset").unwrap();
    let cases = [
        (
            ScriptStore::new([LoadStep::Ready(Ok(Some(cross_record)))]),
            context(&session_b, &fixture.incarnation_id),
            fixture.handle.clone(),
        ),
        (
            ScriptStore::new([LoadStep::Ready(Ok(Some(fixture.record.clone())))]),
            context(&fixture.session_id, &wrong_incarnation),
            fixture.handle.clone(),
        ),
        (
            ScriptStore::new([LoadStep::Ready(Ok(Some(fixture.record.clone())))]),
            fixture.context(),
            synthetic_handle('f'),
        ),
    ];
    for (store, tool_context, handle) in cases {
        let error = execute(
            &store.reader(),
            tool_context,
            arguments(&handle, 1, 4),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert_not_found(&error);
        assert_eq!(store.probe.loads.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn utf8_ranges_accept_boundaries_backtrack_ends_and_distinguish_eof() {
    let fixture = Fixture::new(
        "utf8-session",
        "utf8-incarnation",
        "unicode-call",
        ToolOutput::success("Aé🙂Z"),
    );
    let source = std::str::from_utf8(&fixture.serialized).unwrap();
    let accent = source.find('é').unwrap();
    let emoji = source.find('🙂').unwrap();
    assert_eq!(&source[accent..], "é🙂Z\",\"is_error\":false}");
    let store = ScriptStore::new((0..5).map(|_| LoadStep::Ready(Ok(Some(fixture.record.clone())))));
    let tool = store.reader();

    let exact = execute(
        &tool,
        fixture.context(),
        arguments(&fixture.handle, emoji + 1, 4),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(page_text(&exact), "🙂");
    assert_eq!(exact.content["start_byte"], emoji + 1);
    assert_eq!(exact.content["end_byte"], emoji + 4);

    let backtracked = execute(
        &tool,
        fixture.context(),
        arguments(&fixture.handle, accent + 1, 4),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(page_text(&backtracked), "é");
    assert_eq!(backtracked.content["end_byte"], accent + 2);
    assert_eq!(backtracked.content["has_more"], true);

    let inside_code_point = execute(
        &tool,
        fixture.context(),
        arguments(&fixture.handle, emoji + 2, 4),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_invalid(&inside_code_point);

    let eof = execute(
        &tool,
        fixture.context(),
        arguments(&fixture.handle, source.len() + 1, 4),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(page_text(&eof), "");
    assert_eq!(eof.content["end_byte"], source.len());
    assert_eq!(eof.content["has_more"], false);

    let beyond_eof = execute(
        &tool,
        fixture.context(),
        arguments(&fixture.handle, source.len() + 2, 4),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_invalid(&beyond_eof);
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 5);
}

#[test]
fn scan_count_and_aggregate_byte_limits_are_inclusive_and_fail_closed() {
    let session_id = SessionId::new("scan-session").unwrap();
    let incarnation_id = SessionIncarnationId::new("scan-incarnation").unwrap();
    let entries = [
        (
            ToolCallId::new("scan-1").unwrap(),
            ToolOutput::success("one"),
        ),
        (
            ToolCallId::new("scan-2").unwrap(),
            ToolOutput::success("target"),
        ),
        (
            ToolCallId::new("scan-3").unwrap(),
            ToolOutput::success("outside"),
        ),
    ];
    let record = record_with_results(session_id.clone(), incarnation_id.clone(), entries.clone());
    let serialized = entries
        .iter()
        .map(|(_, output)| serde_json::to_vec(output).unwrap())
        .collect::<Vec<_>>();
    let handles = entries
        .iter()
        .zip(&serialized)
        .map(|((call_id, _), bytes)| handle_for(&session_id, &incarnation_id, call_id, bytes))
        .collect::<Vec<_>>();
    let tool_context = context(&session_id, &incarnation_id);

    let result_store = ScriptStore::new([
        LoadStep::Ready(Ok(Some(record.clone()))),
        LoadStep::Ready(Ok(Some(record.clone()))),
    ]);
    let result_bounded =
        result_store.bounded_reader(ReadToolResultLimits::new(1, 2, MAX_SCAN_BYTES).unwrap());
    execute(
        &result_bounded,
        tool_context.clone(),
        arguments(&handles[1], 1, 4),
        CancellationToken::new(),
    )
    .unwrap();
    assert_not_found(
        &execute(
            &result_bounded,
            tool_context.clone(),
            arguments(&handles[2], 1, 4),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );

    let exact_bytes = serialized[0].len() + serialized[1].len();
    let exact_store = ScriptStore::new([LoadStep::Ready(Ok(Some(record.clone())))]);
    let exact_tool =
        exact_store.bounded_reader(ReadToolResultLimits::new(1, 3, exact_bytes).unwrap());
    execute(
        &exact_tool,
        tool_context.clone(),
        arguments(&handles[1], 1, 4),
        CancellationToken::new(),
    )
    .unwrap();

    let over_store = ScriptStore::new([LoadStep::Ready(Ok(Some(record)))]);
    let over_tool =
        over_store.bounded_reader(ReadToolResultLimits::new(1, 3, exact_bytes - 1).unwrap());
    assert_not_found(
        &execute(
            &over_tool,
            tool_context,
            arguments(&handles[1], 1, 4),
            CancellationToken::new(),
        )
        .unwrap_err(),
    );
    assert_eq!(result_store.probe.loads.load(Ordering::SeqCst), 2);
    assert_eq!(exact_store.probe.loads.load(Ordering::SeqCst), 1);
    assert_eq!(over_store.probe.loads.load(Ordering::SeqCst), 1);
}

#[test]
fn store_failures_preserve_only_retryability_and_redact_identity() {
    let fixture = Fixture::new(
        "secret-session",
        "secret-incarnation",
        "secret-call",
        ToolOutput::success("secret-result"),
    );
    let injected_code = format!("store_code_{}_{}", fixture.session_id, fixture.handle);
    let injected_message = format!(
        "path=/private/sessions/{} incarnation={} result=secret-result",
        fixture.session_id, fixture.incarnation_id
    );
    let store = ScriptStore::new([false, true].map(|retryable| {
        LoadStep::Ready(Err(SessionStoreError::new(
            SessionStoreErrorKind::Corrupt,
            injected_code.clone(),
            injected_message.clone(),
            retryable,
        )))
    }));
    let tool = store.reader();
    for retryable in [false, true] {
        let error = execute(
            &tool,
            fixture.context(),
            arguments(&fixture.handle, 1, 4),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert_error(
            &error,
            ToolErrorKind::Unavailable,
            "read_tool_result_unavailable",
            "tool result store is unavailable",
            retryable,
        );
        let public = format!("{error:?}\n{error}");
        for secret in [
            fixture.session_id.as_str(),
            fixture.incarnation_id.as_str(),
            fixture.handle.as_str(),
            "secret-result",
            "/private/sessions",
            injected_code.as_str(),
        ] {
            assert!(!public.contains(secret), "public error exposed {secret:?}");
        }
    }
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 2);
    assert_eq!(store.probe.drops.load(Ordering::SeqCst), 2);
}

#[test]
fn execution_is_inert_and_cancellation_wins_before_during_and_after_load_poll() {
    let fixture = Fixture::new(
        "cancel-session",
        "cancel-incarnation",
        "cancel-source",
        ToolOutput::success("cancel target"),
    );
    let pending_cancellation = CancellationToken::new();
    let same_poll_cancellation = CancellationToken::new();
    let store = ScriptStore::new([
        LoadStep::Pending,
        LoadStep::CancelThenReady {
            cancellation: same_poll_cancellation.clone(),
            result: Ok(Some(fixture.record.clone())),
        },
    ]);
    let tool = store.reader();
    let args = arguments(&fixture.handle, 1, 4);

    let unpolled = tool.execute(fixture.context(), args.clone(), CancellationToken::new());
    drop(unpolled);
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 0);

    let pre_cancelled = CancellationToken::new();
    pre_cancelled.cancel();
    let error = execute(&tool, fixture.context(), args.clone(), pre_cancelled).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "read_tool_result_cancelled",
        "read_tool_result was cancelled",
        false,
    );
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 0);

    let mut pending = Box::pin(tool.execute(
        fixture.context(),
        args.clone(),
        pending_cancellation.clone(),
    ));
    assert!(poll_once(pending.as_mut()).is_pending());
    assert_eq!(store.probe.live.load(Ordering::SeqCst), 1);
    pending_cancellation.cancel();
    let Poll::Ready(Err(error)) = poll_once(pending.as_mut()) else {
        panic!("pending cancellation did not terminate execution");
    };
    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "read_tool_result_cancelled",
        "read_tool_result was cancelled",
        false,
    );
    drop(pending);
    assert_eq!(store.probe.live.load(Ordering::SeqCst), 0);

    let error = execute(
        &tool,
        fixture.context(),
        args,
        same_poll_cancellation.clone(),
    )
    .unwrap_err();
    assert!(same_poll_cancellation.is_cancelled());
    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "read_tool_result_cancelled",
        "read_tool_result was cancelled",
        false,
    );
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 2);
    assert_eq!(store.probe.drops.load(Ordering::SeqCst), 2);
    assert_eq!(store.probe.live.load(Ordering::SeqCst), 0);
}

#[test]
fn capacity_is_fail_fast_and_recovers_after_success_error_and_drop() {
    let fixture = Fixture::new(
        "capacity-session",
        "capacity-incarnation",
        "capacity-source",
        ToolOutput::success("capacity target"),
    );
    let backend_error = SessionStoreError::new(
        SessionStoreErrorKind::Unavailable,
        "backend-secret",
        "backend secret detail",
        true,
    );
    let store = ScriptStore::new([
        LoadStep::Pending,
        LoadStep::Ready(Ok(Some(fixture.record.clone()))),
        LoadStep::Ready(Err(backend_error)),
        LoadStep::Pending,
        LoadStep::Pending,
    ]);
    let tool = store.bounded_reader(ReadToolResultLimits::new(1, 4_096, MAX_SCAN_BYTES).unwrap());
    let args = arguments(&fixture.handle, 1, 4);

    let unpolled = tool.execute(fixture.context(), args.clone(), CancellationToken::new());
    drop(unpolled);
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 0);

    let mut first =
        Box::pin(tool.execute(fixture.context(), args.clone(), CancellationToken::new()));
    assert!(poll_once(first.as_mut()).is_pending());
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 1);
    assert_eq!(store.probe.live.load(Ordering::SeqCst), 1);

    let busy = execute(
        &tool,
        fixture.context(),
        args.clone(),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_error(
        &busy,
        ToolErrorKind::Unavailable,
        "read_tool_result_busy",
        "read_tool_result is busy",
        true,
    );
    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 1);

    drop(first);
    assert_eq!(store.probe.live.load(Ordering::SeqCst), 0);
    let success = execute(
        &tool,
        fixture.context(),
        args.clone(),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(page_text(&success).len(), 4);

    let unavailable = execute(
        &tool,
        fixture.context(),
        args.clone(),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_error(
        &unavailable,
        ToolErrorKind::Unavailable,
        "read_tool_result_unavailable",
        "tool result store is unavailable",
        true,
    );

    let mut after_error =
        Box::pin(tool.execute(fixture.context(), args.clone(), CancellationToken::new()));
    assert!(poll_once(after_error.as_mut()).is_pending());
    let still_busy = execute(
        &tool,
        fixture.context(),
        args.clone(),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(still_busy.code, "read_tool_result_busy");
    drop(after_error);

    let mut after_drop = Box::pin(tool.execute(fixture.context(), args, CancellationToken::new()));
    assert!(poll_once(after_drop.as_mut()).is_pending());
    drop(after_drop);

    assert_eq!(store.probe.loads.load(Ordering::SeqCst), 5);
    assert_eq!(store.probe.drops.load(Ordering::SeqCst), 5);
    assert_eq!(store.probe.live.load(Ordering::SeqCst), 0);
}
