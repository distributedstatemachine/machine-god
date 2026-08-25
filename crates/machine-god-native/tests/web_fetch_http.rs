#![cfg(all(feature = "web-fetch-http", not(target_family = "wasm")))]

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use machine_god_core::{
    BoxFuture, CancellationToken, SessionId, SessionIncarnationId, Tool, ToolCallId, ToolContext,
    ToolError, ToolErrorKind, ToolOutput, TurnId,
};
use machine_god_native::{
    MAX_WEB_FETCH_BODY_BYTES, WEB_FETCH_DEFAULT_MAX_ACTIVE_REQUESTS, WEB_FETCH_MAX_ACTIVE_REQUESTS,
    WebFetchLimits, WebFetchRequest, WebFetchResponse, WebFetchTool, WebFetchTransport,
    WebFetchTransportError, WebFetchTransportErrorKind,
};
use serde_json::json;

const PRIVATE_QUERY: &str = "PRIVATE_QUERY_MUST_NOT_ESCAPE";

#[derive(Clone)]
enum BoundedMode {
    Text(Vec<u8>),
    TextWithCompletionProbe(Vec<u8>),
    CancelThenText(CancellationToken),
    Error(WebFetchTransportErrorKind),
    Pending,
}

struct BoundedState {
    modes: Mutex<VecDeque<BoundedMode>>,
    calls: AtomicUsize,
    active: AtomicUsize,
    peak_active: AtomicUsize,
    drops: AtomicUsize,
    completion_probe_tool: Mutex<Option<Arc<WebFetchTool>>>,
    completion_probes: AtomicUsize,
}

impl BoundedState {
    fn scripted(modes: impl IntoIterator<Item = BoundedMode>) -> Arc<Self> {
        Arc::new(Self {
            modes: Mutex::new(modes.into_iter().collect()),
            calls: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            peak_active: AtomicUsize::new(0),
            drops: AtomicUsize::new(0),
            completion_probe_tool: Mutex::new(None),
            completion_probes: AtomicUsize::new(0),
        })
    }
}

#[derive(Clone)]
struct BoundedTransport {
    state: Arc<BoundedState>,
}

impl WebFetchTransport for BoundedTransport {
    fn fetch(
        &self,
        request: WebFetchRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<WebFetchResponse, WebFetchTransportError>> {
        assert_eq!(request.url(), "https://example.com/report");
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        let active = self.state.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.state.peak_active.fetch_max(active, Ordering::SeqCst);
        let mode = self
            .state
            .modes
            .lock()
            .unwrap()
            .pop_front()
            .expect("one scripted mode per admitted fetch");
        Box::pin(BoundedFuture {
            mode,
            state: Arc::clone(&self.state),
            completed: false,
        })
    }
}

struct BoundedFuture {
    mode: BoundedMode,
    state: Arc<BoundedState>,
    completed: bool,
}

impl Future for BoundedFuture {
    type Output = Result<WebFetchResponse, WebFetchTransportError>;

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        assert!(!self.completed, "completed transport future was repolled");
        let output = match &self.mode {
            BoundedMode::Text(body) | BoundedMode::TextWithCompletionProbe(body) => Some(
                WebFetchResponse::new(200, Some("text/plain".to_owned()), body.clone()),
            ),
            BoundedMode::CancelThenText(cancellation) => {
                assert!(cancellation.cancel());
                Some(WebFetchResponse::new(
                    200,
                    Some("text/plain".to_owned()),
                    b"must not escape final cancellation".to_vec(),
                ))
            }
            BoundedMode::Error(kind) => Some(Err(WebFetchTransportError::new(*kind))),
            BoundedMode::Pending => None,
        };
        let Some(output) = output else {
            return Poll::Pending;
        };
        self.completed = true;
        Poll::Ready(output)
    }
}

impl Drop for BoundedFuture {
    fn drop(&mut self) {
        self.state.active.fetch_sub(1, Ordering::SeqCst);
        self.state.drops.fetch_add(1, Ordering::SeqCst);
        if self.completed && matches!(self.mode, BoundedMode::TextWithCompletionProbe(_)) {
            let tool = self
                .state
                .completion_probe_tool
                .lock()
                .unwrap()
                .clone()
                .expect("completion probe tool is installed");
            let mut probe = Box::pin(execute_bounded(&tool, CancellationToken::new()));
            assert!(
                poll_once(probe.as_mut()).is_pending(),
                "completion probe must remain queued as response rendering begins"
            );
            assert_eq!(
                self.state.calls.load(Ordering::SeqCst),
                1,
                "capacity was released before response rendering"
            );
            self.state.completion_probes.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("web-fetch-http-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("web-fetch-http-incarnation").unwrap(),
        turn_id: TurnId::new("web-fetch-http-turn").unwrap(),
        call_id: ToolCallId::new("web-fetch-http-call").unwrap(),
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build deterministic test runtime")
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = futures_util::task::noop_waker();
    future.poll(&mut Context::from_waker(&waker))
}

#[derive(Debug, Default)]
struct WakeCounter(AtomicUsize);

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn counting_waker(counter: &Arc<WakeCounter>) -> Waker {
    Waker::from(Arc::clone(counter))
}

fn execute_bounded(
    tool: &WebFetchTool,
    cancellation: CancellationToken,
) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
    tool.execute(
        context(),
        json!({ "url": "https://example.com/report" }),
        cancellation,
    )
}

fn bounded_tool(state: &Arc<BoundedState>, limits: WebFetchLimits) -> WebFetchTool {
    WebFetchTool::with_bounded_transport(
        Arc::new(BoundedTransport {
            state: Arc::clone(state),
        }),
        limits,
    )
}

fn assert_error_code(result: Result<ToolOutput, ToolError>, code: &str) {
    assert_eq!(result.expect_err("execution must fail").code, code);
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

#[test]
fn bounded_transport_enforces_default_and_hard_admission_limits() {
    for (limits, expected_active) in [
        (
            WebFetchLimits::default(),
            WEB_FETCH_DEFAULT_MAX_ACTIVE_REQUESTS,
        ),
        (
            WebFetchLimits::new(Duration::from_secs(1), Duration::from_secs(2), 32).unwrap(),
            WEB_FETCH_MAX_ACTIVE_REQUESTS,
        ),
    ] {
        let state = BoundedState::scripted(std::iter::repeat_n(
            BoundedMode::Pending,
            expected_active + 1,
        ));
        let tool = bounded_tool(&state, limits);
        runtime().block_on(async {
            let mut executions = (0..=expected_active)
                .map(|_| Box::pin(execute_bounded(&tool, CancellationToken::new())))
                .collect::<Vec<_>>();

            for execution in executions.iter_mut().take(expected_active) {
                assert!(poll_once(execution.as_mut()).is_pending());
            }
            assert_eq!(state.calls.load(Ordering::SeqCst), expected_active);
            assert_eq!(state.active.load(Ordering::SeqCst), expected_active);
            assert_eq!(state.peak_active.load(Ordering::SeqCst), expected_active);

            assert!(poll_once(executions[expected_active].as_mut()).is_pending());
            assert_eq!(state.calls.load(Ordering::SeqCst), expected_active);
            assert_eq!(state.peak_active.load(Ordering::SeqCst), expected_active);
            drop(executions);
        });
        assert_eq!(state.active.load(Ordering::SeqCst), 0);
        assert_eq!(state.drops.load(Ordering::SeqCst), expected_active);
    }
}

#[test]
fn bounded_transport_cancels_and_drops_queued_calls_without_admission() {
    let limits = WebFetchLimits::new(Duration::from_secs(1), Duration::from_secs(2), 1).unwrap();
    let state = BoundedState::scripted([
        BoundedMode::Pending,
        BoundedMode::Pending,
        BoundedMode::Pending,
    ]);
    let tool = bounded_tool(&state, limits);

    runtime().block_on(async {
        let first_cancellation = CancellationToken::new();
        let mut first = Box::pin(execute_bounded(&tool, first_cancellation.clone()));
        assert!(poll_once(first.as_mut()).is_pending());
        assert_eq!(state.calls.load(Ordering::SeqCst), 1);

        let queued_cancellation = CancellationToken::new();
        let mut cancelled = Box::pin(execute_bounded(&tool, queued_cancellation.clone()));
        assert!(poll_once(cancelled.as_mut()).is_pending());
        assert!(queued_cancellation.cancel());
        let Poll::Ready(cancelled_result) = poll_once(cancelled.as_mut()) else {
            panic!("queued cancellation must complete promptly")
        };
        assert_error_code(cancelled_result, "web_fetch_cancelled");
        assert_eq!(state.calls.load(Ordering::SeqCst), 1);

        let mut dropped = Box::pin(execute_bounded(&tool, CancellationToken::new()));
        assert!(poll_once(dropped.as_mut()).is_pending());
        drop(dropped);
        assert_eq!(state.calls.load(Ordering::SeqCst), 1);

        assert!(first_cancellation.cancel());
        let Poll::Ready(first_result) = poll_once(first.as_mut()) else {
            panic!("active cancellation must complete promptly")
        };
        assert_error_code(first_result, "web_fetch_cancelled");
        assert_eq!(state.active.load(Ordering::SeqCst), 0);

        let mut replacement = Box::pin(execute_bounded(&tool, CancellationToken::new()));
        assert!(poll_once(replacement.as_mut()).is_pending());
        assert_eq!(state.calls.load(Ordering::SeqCst), 2);
        drop(replacement);
    });
    assert_eq!(state.active.load(Ordering::SeqCst), 0);
    assert_eq!(state.drops.load(Ordering::SeqCst), 2);
}

#[test]
fn bounded_execution_cancellation_wakes_once_and_drops_owned_work() {
    let limits = WebFetchLimits::new(Duration::from_secs(1), Duration::from_secs(60), 1).unwrap();
    let state = BoundedState::scripted([BoundedMode::Pending]);
    let tool = bounded_tool(&state, limits);
    let cancellation = CancellationToken::new();
    let mut execution = Box::pin(execute_bounded(&tool, cancellation.clone()));
    let wake_count = Arc::new(WakeCounter::default());
    let waker = counting_waker(&wake_count);
    let mut context = Context::from_waker(&waker);
    let runtime = runtime();

    {
        let _runtime_guard = runtime.enter();
        assert!(execution.as_mut().poll(&mut context).is_pending());
    }
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.active.load(Ordering::SeqCst), 1);
    assert_eq!(wake_count.0.load(Ordering::SeqCst), 0);

    assert!(cancellation.cancel());
    assert_eq!(
        wake_count.0.load(Ordering::SeqCst),
        1,
        "one bounded execution must register only one cancellation wake"
    );

    let result = {
        let _runtime_guard = runtime.enter();
        let Poll::Ready(result) = execution.as_mut().poll(&mut context) else {
            panic!("bounded cancellation must finish promptly")
        };
        result
    };
    assert_error_code(result, "web_fetch_cancelled");
    assert_eq!(wake_count.0.load(Ordering::SeqCst), 1);
    assert_eq!(state.active.load(Ordering::SeqCst), 0);
    assert_eq!(state.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn raw_execution_cancellation_wakes_once_and_drops_owned_work() {
    let state = BoundedState::scripted([BoundedMode::Pending]);
    let tool = WebFetchTool::with_transport(Arc::new(BoundedTransport {
        state: Arc::clone(&state),
    }));
    let cancellation = CancellationToken::new();
    let mut execution = Box::pin(execute_bounded(&tool, cancellation.clone()));
    let wake_count = Arc::new(WakeCounter::default());
    let waker = counting_waker(&wake_count);
    let mut context = Context::from_waker(&waker);

    assert!(execution.as_mut().poll(&mut context).is_pending());
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.active.load(Ordering::SeqCst), 1);
    assert_eq!(wake_count.0.load(Ordering::SeqCst), 0);

    assert!(cancellation.cancel());
    assert_eq!(wake_count.0.load(Ordering::SeqCst), 1);

    let Poll::Ready(result) = execution.as_mut().poll(&mut context) else {
        panic!("raw cancellation must finish promptly")
    };
    assert_error_code(result, "web_fetch_cancelled");
    assert_eq!(wake_count.0.load(Ordering::SeqCst), 1);
    assert_eq!(state.active.load(Ordering::SeqCst), 0);
    assert_eq!(state.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn bounded_transport_queued_calls_share_one_non_resetting_deadline() {
    let request_timeout = Duration::from_secs(2);
    let limits = WebFetchLimits::new(Duration::from_millis(1), request_timeout, 1).unwrap();
    let state = BoundedState::scripted([BoundedMode::Pending, BoundedMode::Pending]);
    let tool = bounded_tool(&state, limits);

    runtime().block_on(async {
        let first_cancellation = CancellationToken::new();
        let mut first = Box::pin(execute_bounded(&tool, first_cancellation.clone()));
        let mut queued = Box::pin(execute_bounded(&tool, CancellationToken::new()));
        assert!(poll_once(first.as_mut()).is_pending());
        assert!(poll_once(queued.as_mut()).is_pending());
        assert_eq!(state.calls.load(Ordering::SeqCst), 1);

        tokio::time::sleep(Duration::from_millis(1_500)).await;
        assert!(first_cancellation.cancel());
        let Poll::Ready(first_result) = poll_once(first.as_mut()) else {
            panic!("active cancellation must release capacity promptly")
        };
        assert_error_code(first_result, "web_fetch_cancelled");

        let admitted = Instant::now();
        assert_error_code(queued.await, "web_fetch_timeout");
        assert!(
            admitted.elapsed() < Duration::from_millis(1_300),
            "deadline appears to have reset after admission"
        );
        assert_eq!(state.calls.load(Ordering::SeqCst), 2);
    });
    assert_eq!(state.active.load(Ordering::SeqCst), 0);
    assert_eq!(state.drops.load(Ordering::SeqCst), 2);
}

#[test]
fn bounded_transport_times_out_while_queued_without_starting_an_effect() {
    let limits =
        WebFetchLimits::new(Duration::from_millis(1), Duration::from_millis(30), 1).unwrap();
    let state = BoundedState::scripted([BoundedMode::Pending, BoundedMode::Pending]);
    let tool = bounded_tool(&state, limits);

    runtime().block_on(async {
        let first = execute_bounded(&tool, CancellationToken::new());
        let queued = execute_bounded(&tool, CancellationToken::new());
        let (first_result, queued_result) = futures_util::future::join(first, queued).await;
        assert_error_code(first_result, "web_fetch_timeout");
        assert_error_code(queued_result, "web_fetch_timeout");
    });
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.active.load(Ordering::SeqCst), 0);
    assert_eq!(state.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn bounded_transport_releases_capacity_after_success_transport_and_render_errors() {
    for (first_mode, expected_code) in [
        (BoundedMode::Text(b"safe body".to_vec()), None),
        (
            BoundedMode::Error(WebFetchTransportErrorKind::Unavailable),
            Some("web_fetch_unavailable"),
        ),
        (
            BoundedMode::Text(vec![0xff, 0xfe]),
            Some("web_fetch_unsafe_text"),
        ),
    ] {
        let limits =
            WebFetchLimits::new(Duration::from_secs(1), Duration::from_secs(2), 1).unwrap();
        let state = BoundedState::scripted([first_mode, BoundedMode::Pending]);
        let tool = bounded_tool(&state, limits);

        runtime().block_on(async {
            let first_result = execute_bounded(&tool, CancellationToken::new()).await;
            if let Some(code) = expected_code {
                assert_error_code(first_result, code);
            } else {
                assert!(first_result.is_ok());
            }
            assert_eq!(state.active.load(Ordering::SeqCst), 0);
            assert_eq!(state.drops.load(Ordering::SeqCst), 1);

            let mut replacement = Box::pin(execute_bounded(&tool, CancellationToken::new()));
            assert!(poll_once(replacement.as_mut()).is_pending());
            assert_eq!(state.calls.load(Ordering::SeqCst), 2);
            drop(replacement);
        });
        assert_eq!(state.active.load(Ordering::SeqCst), 0);
        assert_eq!(state.drops.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn bounded_transport_retains_capacity_at_fetch_completion_and_releases_after_tool_completion() {
    let limits = WebFetchLimits::new(Duration::from_secs(1), Duration::from_secs(2), 1).unwrap();
    let state = BoundedState::scripted([
        BoundedMode::TextWithCompletionProbe(vec![b'\n'; MAX_WEB_FETCH_BODY_BYTES]),
        BoundedMode::Pending,
    ]);
    let tool = Arc::new(bounded_tool(&state, limits));
    *state.completion_probe_tool.lock().unwrap() = Some(Arc::clone(&tool));

    let output = runtime()
        .block_on(execute_bounded(&tool, CancellationToken::new()))
        .expect("first bounded fetch succeeds after rendering");
    state.completion_probe_tool.lock().unwrap().take();
    assert!(!output.is_error);
    assert_eq!(state.completion_probes.load(Ordering::SeqCst), 1);
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.active.load(Ordering::SeqCst), 0);

    runtime().block_on(async {
        let mut replacement = Box::pin(execute_bounded(&tool, CancellationToken::new()));
        assert!(poll_once(replacement.as_mut()).is_pending());
        assert_eq!(state.calls.load(Ordering::SeqCst), 2);
        drop(replacement);
    });
    assert_eq!(state.drops.load(Ordering::SeqCst), 2);
}

#[test]
fn bounded_transport_releases_capacity_when_cancellation_races_ready_response() {
    let limits = WebFetchLimits::new(Duration::from_secs(1), Duration::from_secs(2), 1).unwrap();
    let cancellation = CancellationToken::new();
    let state = BoundedState::scripted([
        BoundedMode::CancelThenText(cancellation.clone()),
        BoundedMode::Pending,
    ]);
    let tool = bounded_tool(&state, limits);

    runtime().block_on(async {
        assert_error_code(
            execute_bounded(&tool, cancellation).await,
            "web_fetch_cancelled",
        );
        assert_eq!(state.active.load(Ordering::SeqCst), 0);
        assert_eq!(state.drops.load(Ordering::SeqCst), 1);

        let mut replacement = Box::pin(execute_bounded(&tool, CancellationToken::new()));
        assert!(poll_once(replacement.as_mut()).is_pending());
        assert_eq!(state.calls.load(Ordering::SeqCst), 2);
        drop(replacement);
    });
    assert_eq!(state.active.load(Ordering::SeqCst), 0);
    assert_eq!(state.drops.load(Ordering::SeqCst), 2);
}

#[test]
fn bounded_transport_releases_capacity_when_active_execution_is_dropped() {
    let limits = WebFetchLimits::new(Duration::from_secs(1), Duration::from_secs(2), 1).unwrap();
    let state = BoundedState::scripted([BoundedMode::Pending, BoundedMode::Pending]);
    let tool = bounded_tool(&state, limits);

    runtime().block_on(async {
        let mut first = Box::pin(execute_bounded(&tool, CancellationToken::new()));
        assert!(poll_once(first.as_mut()).is_pending());
        assert_eq!(state.calls.load(Ordering::SeqCst), 1);
        drop(first);
        assert_eq!(state.active.load(Ordering::SeqCst), 0);

        let mut replacement = Box::pin(execute_bounded(&tool, CancellationToken::new()));
        assert!(poll_once(replacement.as_mut()).is_pending());
        assert_eq!(state.calls.load(Ordering::SeqCst), 2);
        drop(replacement);
    });
    assert_eq!(state.active.load(Ordering::SeqCst), 0);
    assert_eq!(state.drops.load(Ordering::SeqCst), 2);
}
