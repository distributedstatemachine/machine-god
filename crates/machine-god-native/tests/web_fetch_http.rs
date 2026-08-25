#![cfg(all(feature = "web-fetch-http", not(target_family = "wasm")))]

use std::time::Duration;

use machine_god_core::{
    CancellationToken, SessionId, SessionIncarnationId, Tool, ToolCallId, ToolContext,
    ToolErrorKind, TurnId,
};
use machine_god_native::{WebFetchLimits, WebFetchTool};
use serde_json::json;

const PRIVATE_QUERY: &str = "PRIVATE_QUERY_MUST_NOT_ESCAPE";

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("web-fetch-http-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("web-fetch-http-incarnation").unwrap(),
        turn_id: TurnId::new("web-fetch-http-turn").unwrap(),
        call_id: ToolCallId::new("web-fetch-http-call").unwrap(),
    }
}

#[test]
fn production_transport_construction_is_runtime_independent() {
    WebFetchTool::new().expect("default construction must not require a host runtime");

    let limits = WebFetchLimits::new(Duration::from_secs(1), Duration::from_secs(2), 1)
        .expect("valid narrow production limits");
    WebFetchTool::with_limits(limits)
        .expect("custom-limit construction must not require a host runtime");
}

#[test]
fn production_transport_requires_a_host_runtime_before_resolution() {
    let tool = WebFetchTool::new().expect("construct production transport");
    let error = futures_executor::block_on(tool.execute(
        context(),
        json!({
            "url": format!("https://example.com/report?token={PRIVATE_QUERY}"),
        }),
        CancellationToken::new(),
    ))
    .expect_err("execution outside Tokio must fail before DNS or HTTP");

    assert_eq!(error.kind, ToolErrorKind::Unavailable);
    assert_eq!(error.code, "web_fetch_runtime_required");
    assert_eq!(error.message, "web_fetch requires an active Tokio runtime");
    assert!(!error.retryable);
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains(PRIVATE_QUERY));
    assert!(!diagnostic.contains("example.com"));
}

#[test]
fn production_transport_honors_pre_cancellation_before_runtime_or_resolution() {
    let tool = WebFetchTool::new().expect("construct production transport");
    let cancellation = CancellationToken::new();
    assert!(cancellation.cancel());

    let error = futures_executor::block_on(tool.execute(
        context(),
        json!({
            "url": format!("https://example.com/report?token={PRIVATE_QUERY}"),
        }),
        cancellation,
    ))
    .expect_err("pre-cancellation must win before runtime or DNS");

    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "web_fetch_cancelled");
    assert_eq!(error.message, "web_fetch execution was cancelled");
    assert!(!error.retryable);
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains(PRIVATE_QUERY));
    assert!(!diagnostic.contains("example.com"));
}
