use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_executor::block_on;
use machine_god_core::{
    BoxFuture, CancellationToken, SessionId, SessionIncarnationId, SessionRecord, SessionRevision,
    SessionStore, SessionStoreError, Tool, ToolCall, ToolCallId, ToolContext, ToolError,
    ToolErrorKind, ToolName, TurnId,
};
use machine_god_native::{READ_TOOL_RESULT_TOOL_NAME, ReadToolResultTool};
use serde_json::{Map, Value, json};

const HUGE_ARGUMENT_BYTES: usize = 4 * 1024 * 1024;
const DEEP_ARGUMENT_DEPTH: usize = 10_000;

#[derive(Clone, Default)]
struct LoadProbe {
    loads: Arc<AtomicUsize>,
}

impl LoadProbe {
    fn load_count(&self) -> usize {
        self.loads.load(Ordering::SeqCst)
    }
}

impl SessionStore for LoadProbe {
    fn load(
        &self,
        _id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(None) })
    }

    fn save(
        &self,
        _record: SessionRecord,
        _expected_revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        Box::pin(async { panic!("argument rejection must not save a session") })
    }
}

fn tool(probe: &LoadProbe) -> ReadToolResultTool {
    ReadToolResultTool::shared_session_store(Arc::new(probe.clone()))
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("argument-regression-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("argument-regression-incarnation")
            .unwrap(),
        turn_id: TurnId::new("argument-regression-turn").unwrap(),
        call_id: ToolCallId::new("argument-regression-call").unwrap(),
    }
}

fn call(arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("argument-regression-call").unwrap(),
        name: ToolName::new(READ_TOOL_RESULT_TOOL_NAME).unwrap(),
        arguments,
    }
}

fn handle() -> String {
    format!("tool-result-sha256-{}", "a".repeat(64))
}

fn huge_unknown_value() -> Value {
    json!({
        "handle": handle(),
        "unknown": "x".repeat(HUGE_ARGUMENT_BYTES),
    })
}

fn huge_unknown_key() -> Value {
    let mut object = Map::new();
    object.insert("handle".to_owned(), Value::String(handle()));
    object.insert("k".repeat(HUGE_ARGUMENT_BYTES), Value::Null);
    Value::Object(object)
}

fn deeply_nested_value(depth: usize) -> Value {
    let mut value = Value::Null;
    for _ in 0..depth {
        value = Value::Array(vec![value]);
    }
    value
}

fn assert_resource_limit(error: &ToolError) {
    assert_eq!(error.kind, ToolErrorKind::InvalidInput);
    assert_eq!(error.code, "read_tool_result_resource_limit");
    assert_eq!(error.message, "read_tool_result resource limit exceeded");
    assert!(!error.retryable);
    assert_eq!(
        error.to_string(),
        "read_tool_result_resource_limit: read_tool_result resource limit exceeded"
    );
}

fn assert_invalid(error: &ToolError) {
    assert_eq!(error.kind, ToolErrorKind::InvalidInput);
    assert_eq!(error.code, "read_tool_result_invalid_arguments");
    assert_eq!(error.message, "read_tool_result arguments are invalid");
    assert!(!error.retryable);
}

fn assert_cancelled(error: &ToolError) {
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "read_tool_result_cancelled");
    assert_eq!(error.message, "read_tool_result was cancelled");
    assert!(!error.retryable);
}

#[test]
fn huge_unescaped_invalid_inputs_hit_the_argument_resource_bound_without_store_work() {
    let probe = LoadProbe::default();
    let reader = tool(&probe);
    let oversized = [
        huge_unknown_value(),
        huge_unknown_key(),
        Value::String("s".repeat(HUGE_ARGUMENT_BYTES)),
        json!({"handle": "h".repeat(HUGE_ARGUMENT_BYTES)}),
    ];

    for arguments in oversized {
        assert_resource_limit(&reader.prepare(call(arguments)).unwrap_err());
        assert_eq!(probe.load_count(), 0);
    }

    let error = block_on(reader.execute(context(), huge_unknown_value(), CancellationToken::new()))
        .unwrap_err();
    assert_resource_limit(&error);
    assert_eq!(probe.load_count(), 0);
}

#[test]
fn deeply_nested_invalid_values_have_fixed_errors_and_nonrecursive_ownership() {
    let probe = LoadProbe::default();
    let reader = tool(&probe);

    let shallow_error = reader.prepare(call(deeply_nested_value(64))).unwrap_err();
    assert_invalid(&shallow_error);
    assert_eq!(probe.load_count(), 0);

    let deep_prepare = reader
        .prepare(call(deeply_nested_value(DEEP_ARGUMENT_DEPTH)))
        .unwrap_err();
    assert_resource_limit(&deep_prepare);
    assert_eq!(probe.load_count(), 0);

    let deep_execute = block_on(reader.execute(
        context(),
        deeply_nested_value(DEEP_ARGUMENT_DEPTH),
        CancellationToken::new(),
    ))
    .unwrap_err();
    assert_resource_limit(&deep_execute);
    assert_eq!(probe.load_count(), 0);

    let mut wrong_name = call(deeply_nested_value(DEEP_ARGUMENT_DEPTH));
    wrong_name.name = ToolName::new("not_read_tool_result").unwrap();
    assert_invalid(&reader.prepare(wrong_name).unwrap_err());
    assert_eq!(probe.load_count(), 0);
}

#[test]
fn cancellation_wins_before_invalid_normalization_and_unpolled_execution_is_inert() {
    let probe = LoadProbe::default();
    let reader = tool(&probe);
    let cancellation = CancellationToken::new();
    let future = reader.execute(context(), huge_unknown_value(), cancellation.clone());
    assert_eq!(probe.load_count(), 0);

    cancellation.cancel();
    let error = block_on(future).unwrap_err();
    assert_cancelled(&error);
    assert_eq!(probe.load_count(), 0);

    drop(reader.execute(
        context(),
        deeply_nested_value(DEEP_ARGUMENT_DEPTH),
        CancellationToken::new(),
    ));
    assert_eq!(probe.load_count(), 0);

    let deep_cancellation = CancellationToken::new();
    deep_cancellation.cancel();
    let error = block_on(reader.execute(
        context(),
        deeply_nested_value(DEEP_ARGUMENT_DEPTH),
        deep_cancellation,
    ))
    .unwrap_err();
    assert_cancelled(&error);
    assert_eq!(probe.load_count(), 0);
}
