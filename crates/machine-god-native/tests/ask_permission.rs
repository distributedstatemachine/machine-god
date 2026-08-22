use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use futures_executor::block_on;
use futures_util::future;
use machine_god_core::{
    BoxFuture, Capability, PermissionDecision, PermissionError, PermissionGrantScope,
    PermissionHandler, PermissionRequest, PermissionRequestId, PermissionRisk, SessionId,
    SessionIncarnationId, TurnId,
};
use machine_god_native::{
    ASK_PERMISSION_DENIED_REASON, ASK_PERMISSION_PROMPT_ERROR_CODE,
    ASK_PERMISSION_PROMPT_ERROR_MESSAGE, AskPermissionHandler, PermissionPromptDecision,
    PermissionPromptError, PermissionPrompter,
};
use serde_json::json;

type PromptResult = Result<PermissionPromptDecision, PermissionPromptError>;

#[derive(Debug)]
struct ScriptedState {
    results: VecDeque<PromptResult>,
    requests: Vec<PermissionRequest>,
}

#[derive(Clone, Debug)]
struct ScriptedPrompter {
    state: Arc<Mutex<ScriptedState>>,
}

impl ScriptedPrompter {
    fn new(results: impl IntoIterator<Item = PromptResult>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ScriptedState {
                results: results.into_iter().collect(),
                requests: Vec::new(),
            })),
        }
    }

    fn requests(&self) -> Vec<PermissionRequest> {
        self.state
            .lock()
            .expect("scripted prompter mutex should not be poisoned")
            .requests
            .clone()
    }
}

impl PermissionPrompter for ScriptedPrompter {
    fn prompt(&self, request: PermissionRequest) -> BoxFuture<'_, PromptResult> {
        let result = {
            let mut state = self
                .state
                .lock()
                .expect("scripted prompter mutex should not be poisoned");
            state.requests.push(request);
            state
                .results
                .pop_front()
                .expect("scripted prompter should have another result")
        };
        Box::pin(future::ready(result))
    }
}

#[derive(Debug, Default)]
struct PendingProbe {
    prompts: AtomicUsize,
    futures_created: AtomicUsize,
    polls: AtomicUsize,
    futures_dropped: AtomicUsize,
    live_futures: AtomicUsize,
}

#[derive(Clone, Debug)]
struct PendingPrompter {
    probe: Arc<PendingProbe>,
}

impl PermissionPrompter for PendingPrompter {
    fn prompt(&self, _request: PermissionRequest) -> BoxFuture<'_, PromptResult> {
        self.probe.prompts.fetch_add(1, Ordering::SeqCst);
        Box::pin(TrackedPendingPrompt::new(Arc::clone(&self.probe)))
    }
}

#[derive(Debug)]
struct TrackedPendingPrompt {
    probe: Arc<PendingProbe>,
}

impl TrackedPendingPrompt {
    fn new(probe: Arc<PendingProbe>) -> Self {
        probe.futures_created.fetch_add(1, Ordering::SeqCst);
        probe.live_futures.fetch_add(1, Ordering::SeqCst);
        Self { probe }
    }
}

impl Future for TrackedPendingPrompt {
    type Output = PromptResult;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.probe.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for TrackedPendingPrompt {
    fn drop(&mut self) {
        self.probe.futures_dropped.fetch_add(1, Ordering::SeqCst);
        self.probe.live_futures.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

#[derive(Clone, Debug, Default)]
struct RoutingPrompter {
    requests: Arc<Mutex<Vec<PermissionRequest>>>,
}

impl RoutingPrompter {
    fn requests(&self) -> Vec<PermissionRequest> {
        self.requests
            .lock()
            .expect("routing prompter mutex should not be poisoned")
            .clone()
    }
}

impl PermissionPrompter for RoutingPrompter {
    fn prompt(&self, request: PermissionRequest) -> BoxFuture<'_, PromptResult> {
        let decision = match request.id.as_str() {
            "permission-concurrent-first" => PermissionPromptDecision::AllowTurn,
            "permission-concurrent-second" => PermissionPromptDecision::Deny,
            other => panic!("unexpected permission request ID: {other}"),
        };
        self.requests
            .lock()
            .expect("routing prompter mutex should not be poisoned")
            .push(request);
        Box::pin(future::ready(Ok(decision)))
    }
}

#[derive(Clone, Debug)]
struct SecretFailingPrompter {
    diagnostic: &'static str,
}

impl PermissionPrompter for SecretFailingPrompter {
    fn prompt(&self, _request: PermissionRequest) -> BoxFuture<'_, PromptResult> {
        assert!(
            !self.diagnostic.is_empty(),
            "test diagnostic must be present"
        );
        Box::pin(future::ready(Err(PermissionPromptError::new())))
    }
}

fn request(suffix: &str) -> PermissionRequest {
    PermissionRequest {
        id: PermissionRequestId::new(format!("permission-{suffix}"))
            .expect("test permission request ID should be valid"),
        session_id: SessionId::new(format!("session-{suffix}"))
            .expect("test session ID should be valid"),
        session_incarnation_id: SessionIncarnationId::new(format!("incarnation-{suffix}"))
            .expect("test session incarnation ID should be valid"),
        turn_id: TurnId::new(format!("turn-{suffix}")).expect("test turn ID should be valid"),
        capability: Capability::Custom {
            name: format!("custom-{suffix}"),
            details: json!({
                "nested": {"opaque": suffix},
                "sequence": [3, 1, 4],
            }),
        },
        risk: PermissionRisk::Critical,
        reason: format!("exact opaque reason for {suffix}"),
    }
}

#[test]
fn forwards_the_exact_owned_request_to_the_prompt() {
    let prompter = ScriptedPrompter::new([Ok(PermissionPromptDecision::AllowOnce)]);
    let handler = AskPermissionHandler::new(prompter.clone());
    let expected = request("forwarded");

    let decision = block_on(handler.authorize(expected.clone()))
        .expect("scripted permission prompt should allow the request");

    assert_eq!(
        decision,
        PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        }
    );
    assert_eq!(prompter.requests(), vec![expected]);
}

#[test]
fn maps_each_positive_prompt_decision_to_its_core_scope() {
    let cases = [
        (
            PermissionPromptDecision::AllowOnce,
            PermissionGrantScope::Once,
        ),
        (
            PermissionPromptDecision::AllowTurn,
            PermissionGrantScope::Turn,
        ),
        (
            PermissionPromptDecision::AllowSession,
            PermissionGrantScope::Session,
        ),
    ];

    for (index, (prompt_decision, expected_scope)) in cases.into_iter().enumerate() {
        let prompter = ScriptedPrompter::new([Ok(prompt_decision)]);
        let handler = AskPermissionHandler::new(prompter);

        let decision = block_on(handler.authorize(request(&format!("allow-{index}"))))
            .expect("positive prompt decision should authorize the request");

        assert_eq!(
            decision,
            PermissionDecision::Allow {
                scope: expected_scope,
            }
        );
    }
}

#[test]
fn maps_prompt_denial_to_the_fixed_operator_neutral_reason() {
    let prompter = ScriptedPrompter::new([Ok(PermissionPromptDecision::Deny)]);
    let handler = AskPermissionHandler::new(prompter);

    let decision = block_on(handler.authorize(request("denied")))
        .expect("a denial is a successful policy decision");

    assert_eq!(ASK_PERMISSION_DENIED_REASON, "permission denied");
    assert_eq!(
        decision,
        PermissionDecision::Deny {
            reason: ASK_PERMISSION_DENIED_REASON.to_owned(),
        }
    );
}

#[test]
#[allow(clippy::default_constructed_unit_structs)]
fn every_prompt_failure_maps_to_one_stable_redacted_core_error() {
    assert_eq!(PermissionPromptError::new(), PermissionPromptError);
    assert_eq!(PermissionPromptError::default(), PermissionPromptError);
    assert_eq!(
        format!("{}", PermissionPromptError::new()),
        ASK_PERMISSION_PROMPT_ERROR_MESSAGE
    );
    assert_eq!(
        format!("{:?}", PermissionPromptError::new()),
        "PermissionPromptError"
    );

    let prompter = ScriptedPrompter::new([
        Err(PermissionPromptError::new()),
        Err(PermissionPromptError::default()),
    ]);
    let handler = AskPermissionHandler::new(prompter);

    for suffix in ["error-new", "error-default"] {
        let error = block_on(handler.authorize(request(suffix)))
            .expect_err("prompt failure must fail closed");
        let expected = PermissionError::new(
            ASK_PERMISSION_PROMPT_ERROR_CODE,
            ASK_PERMISSION_PROMPT_ERROR_MESSAGE,
        );

        assert_eq!(error, expected);
        assert_eq!(error.code, "permission_prompt_failed");
        assert_eq!(error.message, "permission prompt failed");
        assert_eq!(
            error.to_string(),
            "permission_prompt_failed: permission prompt failed"
        );
    }
}

#[test]
fn authorization_is_inert_until_polled() {
    let prompter = ScriptedPrompter::new([Ok(PermissionPromptDecision::AllowOnce)]);
    let handler = AskPermissionHandler::new(prompter.clone());

    let authorization = handler.authorize(request("never-polled"));
    assert!(prompter.requests().is_empty());
    drop(authorization);
    assert!(prompter.requests().is_empty());

    let authorization = handler.authorize(request("polled"));
    assert!(prompter.requests().is_empty());
    let result = block_on(authorization);
    assert!(result.is_ok());
    assert_eq!(prompter.requests(), vec![request("polled")]);
}

#[test]
fn dropping_authorization_drops_the_pending_prompt_without_detached_work() {
    let probe = Arc::new(PendingProbe::default());
    let handler = AskPermissionHandler::new(PendingPrompter {
        probe: Arc::clone(&probe),
    });
    let mut authorization = handler.authorize(request("pending"));

    assert_eq!(probe.prompts.load(Ordering::SeqCst), 0);
    assert_eq!(probe.futures_created.load(Ordering::SeqCst), 0);
    assert_eq!(probe.live_futures.load(Ordering::SeqCst), 0);

    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    assert!(matches!(
        authorization.as_mut().poll(&mut context),
        Poll::Pending
    ));
    assert_eq!(probe.prompts.load(Ordering::SeqCst), 1);
    assert_eq!(probe.futures_created.load(Ordering::SeqCst), 1);
    assert_eq!(probe.polls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.live_futures.load(Ordering::SeqCst), 1);
    assert_eq!(probe.futures_dropped.load(Ordering::SeqCst), 0);

    drop(authorization);

    assert_eq!(probe.futures_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(probe.live_futures.load(Ordering::SeqCst), 0);
}

#[test]
fn concurrent_authorizations_keep_requests_and_results_distinct() {
    let prompter = RoutingPrompter::default();
    let handler = AskPermissionHandler::new(prompter.clone());
    let first_request = request("concurrent-first");
    let second_request = request("concurrent-second");

    let first = handler.authorize(first_request.clone());
    let second = handler.authorize(second_request.clone());
    assert!(prompter.requests().is_empty());

    let (first_result, second_result) = block_on(future::join(first, second));

    assert_eq!(
        first_result,
        Ok(PermissionDecision::Allow {
            scope: PermissionGrantScope::Turn,
        })
    );
    assert_eq!(
        second_result,
        Ok(PermissionDecision::Deny {
            reason: ASK_PERMISSION_DENIED_REASON.to_owned(),
        })
    );
    assert_eq!(prompter.requests(), vec![first_request, second_request]);
}

#[test]
fn debug_and_failures_do_not_expose_prompter_diagnostics() {
    const SECRET: &str = "terminal-error: API_TOKEN=do-not-log-this";
    let handler = AskPermissionHandler::new(SecretFailingPrompter { diagnostic: SECRET });

    let handler_debug = format!("{handler:?}");
    assert_eq!(handler_debug, "AskPermissionHandler { .. }");
    assert!(!handler_debug.contains(SECRET));

    let error = block_on(handler.authorize(request("secret")))
        .expect_err("secret-bearing host failure must fail closed");
    assert!(!format!("{error:?}").contains(SECRET));
    assert!(!error.to_string().contains(SECRET));
    assert_eq!(error.message, ASK_PERMISSION_PROMPT_ERROR_MESSAGE);
}

#[test]
fn handler_supports_shared_trait_objects_and_send_futures() {
    fn assert_send_sync_static<T: Send + Sync + 'static>() {}
    fn assert_send<T: Send>(value: T) {
        drop(value);
    }

    assert_send_sync_static::<AskPermissionHandler>();
    assert_send_sync_static::<Arc<dyn PermissionPrompter>>();

    let concrete = ScriptedPrompter::new([Ok(PermissionPromptDecision::AllowSession)]);
    let shared_prompter: Arc<dyn PermissionPrompter> = Arc::new(concrete.clone());
    let handler = AskPermissionHandler::shared_prompter(shared_prompter);
    let policy: Arc<dyn PermissionHandler> = Arc::new(handler);

    assert_send(policy.authorize(request("send")));
    assert!(concrete.requests().is_empty());
}
