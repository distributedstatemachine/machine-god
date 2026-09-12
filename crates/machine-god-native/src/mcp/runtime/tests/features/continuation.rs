//! Pure actual human-command/ScriptPeer composition; the presenter supplies
//! schema-validated answer data, never a tool grant or a synthetic `ToolContext`.

use super::*;
use crate::mcp::{
    feature::McpFeatureOutcome,
    interaction::{
        McpElicitationAnswer, McpElicitationAnswerInput, McpElicitationPresenter,
        McpElicitationPromptError, McpElicitationPromptRequest, McpElicitationPromptSource,
    },
};
use serde_json::value::RawValue;
use std::task::{Poll, Waker};

const FORM: &str = r#""inputRequests":{"confirm":{"method":"elicitation/create","params":{"message":"Continue?","requestedSchema":{"type":"object","properties":{"amount":{"type":"number"}},"required":["amount"]}}}}"#;
const READ: &str = "resource read fixture test://fixed";
const GET: &str = r#"prompt get fixture review {"topic":"unchanged \"text\""}"#;

#[derive(Default)]
struct Presenter {
    requests: Mutex<Vec<McpElicitationPromptRequest>>,
    gate: Option<CancellationToken>,
    revoke_after_answer: Option<CancellationToken>,
}
impl McpElicitationPresenter for Presenter {
    fn present(
        &self,
        request: McpElicitationPromptRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<McpElicitationAnswer, McpElicitationPromptError>> {
        Box::pin(async move {
            let input = McpElicitationAnswerInput::new(
                RawValue::from_string(
                    r#"{"action":"accept","content":{"amount":9007199254740993.00001}}"#.into(),
                )
                .unwrap(),
            )
            .unwrap();
            let answer = McpElicitationAnswer::validate(request.request(), &input)?;
            {
                let mut requests = self.requests.lock().unwrap();
                assert!(requests.len() < 8, "bounded fixture presenter");
                requests.push(request);
            }
            if let Some(gate) = &self.gate {
                gate.cancelled().await;
            }
            if cancellation.is_cancelled() {
                return Err(McpElicitationPromptError::Cancelled);
            }
            if let Some(revoke) = &self.revoke_after_answer {
                revoke.cancel();
            }
            Ok(answer)
        })
    }
}

fn source() -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        SessionId::new("human-session").unwrap(),
        SessionIncarnationId::new("human-incarnation").unwrap(),
    )
}

fn interactive(presenter: Arc<Presenter>) -> Arc<NativeMcpRuntime> {
    Arc::new(standalone().with_feature_input(Some(presenter), None))
}

fn install_input(
    runtime: &NativeMcpRuntime,
    input: String,
    repeat: bool,
    guard: Option<CancellationToken>,
) -> Arc<Mutex<Vec<u8>>> {
    let writes = Arc::<Mutex<Vec<u8>>>::default();
    let wire = writes.clone();
    let rounds = AtomicUsize::new(0);
    let mut peer = script::ScriptPeer::new(writes.clone()).with_response(move |id| {
        let requests = sent(&wire);
        let request = requests.last().unwrap();
        let method = request["method"].as_str().unwrap();
        let body = match method {
            "resources/list" => r#""resources":[{"uri":"test://fixed","name":"fixed"}]"#,
            "prompts/list" => r#""prompts":[{"name":"review","arguments":[{"name":"topic","required":true}]}]"#,
            "resources/read" | "prompts/get"
                if rounds.fetch_add(1, Ordering::SeqCst) == 0 || repeat =>
            {
                return format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"result":{{"resultType":"input_required",{input}}}}}"#
                )
                .into_bytes()
                .into();
            }
            "resources/read" => r#""contents":[{"uri":"test://fixed","text":"finished"}]"#,
            "prompts/get" => r#""messages":[{"role":"assistant","content":{"type":"text","text":"finished"}}]"#,
            _ => panic!("unexpected feature method {method}"),
        };
        format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"resultType":"complete",{body}}}}}"#)
            .into_bytes()
            .into()
    });
    peer.tools = false;
    let candidate = runtime
        .prepare_candidate(
            vec![NativeMcpServerCandidate {
                server: Arc::from("fixture"),
                configuration: Arc::from(&b"configuration"[..]),
                authentication: Arc::from(&b"authentication"[..]),
                catalogs: vec![],
                catalog_epoch: runtime.clock.now(),
                peer: NativeMcpOwnedPeer::Script(peer),
                operation_timeout: Duration::from_secs(120),
                authority_cancellations: guard.into_iter().collect(),
            }],
            &[],
        )
        .unwrap();
    runtime.publish(candidate).unwrap();
    writes
}

fn unresolved(result: &NativeMcpFeatureResult) {
    let McpFeatureReply::Response(response) = result.reply() else {
        panic!("expected feature response")
    };
    assert!(matches!(
        response.outcome(),
        McpFeatureOutcome::UnvalidatedInputRequired
    ));
}

#[test]
fn read_and_get_resume_exact_params_state_numbers_metadata_and_human_origin() {
    for command in [READ, GET] {
        for state in [
            None,
            Some("null"),
            Some(r#"{"n":1e-99999,"v":9007199254740993.00001}"#),
        ] {
            let presenter = Arc::new(Presenter::default());
            let runtime = interactive(presenter.clone());
            let mut input = FORM.to_owned();
            if let Some(state) = state {
                input.push_str(r#", "requestState":"#);
                input.push_str(state);
            }
            let writes = install_input(&runtime, input, false, None);
            let query = request(command);
            let result = futures_executor::block_on(runtime.human_command().feature_interactive(
                &query,
                CancellationToken::new(),
                &source(),
            ))
            .unwrap();
            let McpFeatureReply::Response(response) = result.reply() else {
                panic!("expected feature response")
            };
            assert!(matches!(
                response.outcome(),
                McpFeatureOutcome::Resource { .. } | McpFeatureOutcome::Prompt { .. }
            ));
            let requests = sent(&writes);
            assert_eq!(requests.len(), 3, "one catalog load, initial call, resume");
            for (index, request) in requests.iter().enumerate() {
                assert_eq!(
                    request["id"].as_u64(),
                    Some(u64::try_from(index).unwrap() + 1)
                );
            }
            assert_eq!(requests[1]["method"], requests[2]["method"]);
            let mut resumed = requests[2]["params"].clone();
            let object = resumed.as_object_mut().unwrap();
            let answers = object.remove("inputResponses").unwrap();
            assert_eq!(
                answers["confirm"]["content"]["amount"].to_string(),
                "9007199254740993.00001"
            );
            assert_eq!(
                object.remove("requestState").map(|value| value.to_string()),
                state.map(str::to_owned)
            );
            assert_eq!(resumed, requests[1]["params"]);
            let metadata = &resumed["_meta"];
            assert!(
                metadata["io.modelcontextprotocol/clientCapabilities"]["elicitation"]["form"]
                    .is_object()
            );
            let prompts = presenter.requests.lock().unwrap();
            assert_eq!(prompts.len(), 1);
            assert_eq!(prompts[0].server(), "fixture");
            let McpElicitationPromptSource::HumanFeature { owner, action } = prompts[0].source()
            else {
                panic!("human command must never synthesize a model source")
            };
            assert_eq!(owner, &source());
            assert_eq!(*action, query.action());
        }
    }
}

#[test]
fn eight_resumptions_leave_the_ninth_input_unresolved_without_rediscovery() {
    let presenter = Arc::new(Presenter::default());
    let runtime = interactive(presenter.clone());
    let writes = install_input(&runtime, FORM.into(), true, None);
    let result = futures_executor::block_on(runtime.human_command().feature_interactive(
        &request(READ),
        CancellationToken::new(),
        &source(),
    ))
    .unwrap();
    unresolved(&result);
    assert_eq!(presenter.requests.lock().unwrap().len(), 8);
    let requests = sent(&writes);
    assert_eq!(requests.len(), 10);
    assert_eq!(requests[0]["method"], "resources/list");
    for (index, request) in requests.iter().enumerate().skip(1) {
        assert_eq!(request["method"], "resources/read");
        assert_eq!(
            request["id"].as_u64(),
            Some(u64::try_from(index).unwrap() + 1)
        );
        assert_eq!(request["params"].get("inputResponses").is_some(), index > 1);
    }
}

#[test]
fn empty_unsupported_and_unavailable_url_input_remain_unresolved_without_prompting() {
    for input in [
        r#""requestState":null"#,
        r#""inputRequests":{}"#,
        r#""inputRequests":{"roots":{"method":"roots/list"}}"#,
        r#""inputRequests":{"url":{"method":"elicitation/create","params":{"mode":"url","message":"Continue?","url":"https://example.com/continue"}}}"#,
    ] {
        let presenter = Arc::new(Presenter::default());
        let runtime = interactive(presenter.clone());
        let writes = install_input(&runtime, input.into(), false, None);
        let result = futures_executor::block_on(runtime.human_command().feature_interactive(
            &request(READ),
            CancellationToken::new(),
            &source(),
        ))
        .unwrap();
        unresolved(&result);
        assert!(presenter.requests.lock().unwrap().is_empty());
        assert_eq!(sent(&writes).len(), 2);
    }
}

#[test]
fn answer_does_not_override_caller_cancellation_or_selected_authority_revocation() {
    for revoke_guard in [false, true] {
        let revoked = CancellationToken::new();
        let presenter = Arc::new(Presenter {
            revoke_after_answer: Some(revoked.clone()),
            ..Default::default()
        });
        let runtime = interactive(presenter.clone());
        let writes = install_input(
            &runtime,
            FORM.into(),
            false,
            revoke_guard.then(|| revoked.clone()),
        );
        let caller = if revoke_guard {
            CancellationToken::new()
        } else {
            revoked
        };
        assert!(
            futures_executor::block_on(runtime.human_command().feature_interactive(
                &request(READ),
                caller,
                &source(),
            ))
            .is_err()
        );
        assert_eq!(presenter.requests.lock().unwrap().len(), 1);
        assert_eq!(
            sent(&writes).len(),
            2,
            "answer must not write a resumed request"
        );
        assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
    }
}

#[test]
fn pending_humans_release_peer_mutex_but_retain_two_operation_slots() {
    let presenter = Arc::new(Presenter {
        gate: Some(CancellationToken::new()),
        ..Default::default()
    });
    let runtime = interactive(presenter.clone());
    let writes = install_input(&runtime, FORM.into(), true, None);
    let publication = runtime.state.lock().unwrap().active.clone().unwrap();
    let owner = runtime.human_command();
    let source = source();
    let query = request(READ);
    let mut first = Box::pin(owner.feature_interactive(&query, CancellationToken::new(), &source));
    let mut second = Box::pin(owner.feature_interactive(&query, CancellationToken::new(), &source));
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(first.as_mut().poll(&mut cx).is_pending());
    assert!(publication.servers[0].peer.try_lock().is_some());
    assert!(second.as_mut().poll(&mut cx).is_pending());
    assert!(publication.servers[0].peer.try_lock().is_some());
    assert_eq!(presenter.requests.lock().unwrap().len(), 2);
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 2);
    assert!(matches!(
        futures_executor::block_on(owner.feature_interactive(
            &query,
            CancellationToken::new(),
            &source
        )),
        Err(NativeMcpFeatureError::Runtime(NativeMcpRuntimeError::Limit))
    ));
    assert_eq!(sent(&writes).len(), 4);
    owner.close();
    assert!(matches!(first.as_mut().poll(&mut cx), Poll::Ready(Err(_))));
    assert!(matches!(second.as_mut().poll(&mut cx), Poll::Ready(Err(_))));
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
}

#[test]
fn replacement_during_human_wait_never_routes_resume_through_new_publication() {
    let gate = CancellationToken::new();
    let presenter = Arc::new(Presenter {
        gate: Some(gate.clone()),
        ..Default::default()
    });
    let runtime = interactive(presenter);
    let original = install_input(&runtime, FORM.into(), false, None);
    let owner = runtime.human_command();
    let source = source();
    let query = request(READ);
    let mut operation =
        Box::pin(owner.feature_interactive(&query, CancellationToken::new(), &source));
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(operation.as_mut().poll(&mut cx).is_pending());
    let replacement = Arc::<Mutex<Vec<u8>>>::default();
    install(&runtime, replacement.clone(), None);
    gate.cancel();
    assert!(matches!(
        operation.as_mut().poll(&mut cx),
        Poll::Ready(Err(_))
    ));
    assert_eq!(sent(&original).len(), 2);
    assert!(sent(&replacement).is_empty());
}

struct ManualClock {
    now: Mutex<Instant>,
    waiters: Mutex<Vec<Waker>>,
    deadlines: Mutex<Vec<Instant>>,
}
impl ManualClock {
    fn advance(&self, duration: Duration) {
        *self.now.lock().unwrap() += duration;
        let waiters = std::mem::take(&mut *self.waiters.lock().unwrap());
        for waiter in waiters {
            waiter.wake();
        }
    }
}
impl NativeMcpRuntimeClock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        {
            let mut deadlines = self.deadlines.lock().unwrap();
            assert!(
                deadlines.len() < 64,
                "bounded fixture deadline observations"
            );
            deadlines.push(deadline);
        }
        Box::pin(std::future::poll_fn(move |cx| {
            let mut waiters = self.waiters.lock().unwrap();
            if self.now() >= deadline {
                Poll::Ready(())
            } else {
                if !waiters.iter().any(|waker| waker.will_wake(cx.waker())) {
                    assert!(waiters.len() < 16, "bounded fixture clock");
                    waiters.push(cx.waker().clone());
                }
                Poll::Pending
            }
        }))
    }
}

#[test]
fn human_budget_is_thirty_minutes_and_success_gets_a_fresh_network_deadline() {
    for (seconds, queued_seconds) in [(1799, 0), (1799, 119), (1799, 120), (1800, 0)] {
        let gate = CancellationToken::new();
        let presenter = Arc::new(Presenter {
            gate: Some(gate.clone()),
            ..Default::default()
        });
        let initial = Instant::now();
        let clock = Arc::new(ManualClock {
            now: Mutex::new(initial),
            waiters: Mutex::default(),
            deadlines: Mutex::default(),
        });
        let mut runtime = standalone().with_feature_input(Some(presenter), None);
        runtime.clock = clock.clone();
        let runtime = Arc::new(runtime);
        let writes = install_input(&runtime, FORM.into(), false, None);
        let owner = runtime.human_command();
        let source = source();
        let query = request(READ);
        let mut operation =
            Box::pin(owner.feature_interactive(&query, CancellationToken::new(), &source));
        let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
        assert!(operation.as_mut().poll(&mut cx).is_pending());
        assert!(
            clock
                .deadlines
                .lock()
                .unwrap()
                .contains(&(initial + Duration::from_secs(1800)))
        );
        let publication = runtime.state.lock().unwrap().active.clone().unwrap();
        let lane = futures_executor::block_on(publication.servers[0].peer.lock());
        clock.advance(Duration::from_secs(seconds));
        gate.cancel();
        if seconds == 1799 {
            // A ready inert peer never polls its timer. Holding the actual peer
            // lane makes the resumed queue observe its fresh transport deadline.
            assert!(operation.as_mut().poll(&mut cx).is_pending());
            assert!(
                clock
                    .deadlines
                    .lock()
                    .unwrap()
                    .contains(&(initial + Duration::from_secs(seconds + 120)))
            );
            clock.advance(Duration::from_secs(queued_seconds));
        }
        drop(lane);
        let Poll::Ready(result) = operation.as_mut().poll(&mut cx) else {
            panic!("ready answer or expired human/transport budget")
        };
        let successful = seconds == 1799 && queued_seconds < 120;
        assert_eq!(result.is_ok(), successful);
        assert_eq!(sent(&writes).len(), if successful { 3 } else { 2 });
    }
}

#[test]
fn constructing_or_dropping_unpolled_interactive_feature_is_inert() {
    let presenter = Arc::new(Presenter::default());
    let runtime = interactive(presenter.clone());
    let writes = install_input(&runtime, FORM.into(), false, None);
    let owner = runtime.human_command();
    drop(owner.feature_interactive(&request(READ), CancellationToken::new(), &source()));
    assert!(sent(&writes).is_empty());
    assert!(presenter.requests.lock().unwrap().is_empty());
    assert_eq!(runtime.feature_operations.load(Ordering::Acquire), 0);
}
