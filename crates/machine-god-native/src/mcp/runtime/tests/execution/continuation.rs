use super::*;
use crate::mcp::interaction::McpElicitationAnswerInput;
use futures_util::future::{Either, select};
use std::task::{Context, Poll, Waker};

mod client_url;
mod url;

const INPUT: &str = r#""result":{"resultType":"input_required","inputRequests":{"confirm":{"method":"elicitation/create","params":{"message":"Confirm","requestedSchema":{"type":"object","properties":{"number":{"type":"number"}}}}}},"requestState":{"exact":1e-99999,"zero":-0,"$serde_json::private::Number":{"$serde_json::private::RawValue":"literal"}}}"#;

fn configured(
    archive: &Archive,
    arguments: &[Value],
    response: impl Fn(i64) -> Box<[u8]> + Send + Sync + 'static,
) -> (Fixture, NativeInteractivePromptInbox) {
    configured_with_clock(archive, arguments, response, None)
}
fn configured_with_clock(
    archive: &Archive,
    arguments: &[Value],
    response: impl Fn(i64) -> Box<[u8]> + Send + Sync + 'static,
    clock: Option<Arc<dyn NativeMcpRuntimeClock>>,
) -> (Fixture, NativeInteractivePromptInbox) {
    configured_with_launcher(archive, arguments, response, clock, None)
}
fn configured_with_launcher(
    archive: &Archive,
    arguments: &[Value],
    response: impl Fn(i64) -> Box<[u8]> + Send + Sync + 'static,
    clock: Option<Arc<dyn NativeMcpRuntimeClock>>,
    launcher: Option<crate::mcp::browser_launcher::NativeMcpBrowserLauncher>,
) -> (Fixture, NativeInteractivePromptInbox) {
    let (bridge, mut inbox) =
        NativeInteractivePromptBridge::new(NativeInteractivePromptLimits::default()).unwrap();
    inbox
        .activate(BackgroundOutputOwner::new(
            SessionId::new("runtime").unwrap(),
            SessionIncarnationId::new("life").unwrap(),
        ))
        .unwrap();
    let executor = NativeMcpArchivedToolExecutor::new(Arc::new(
        NativeToolResultArchiveAdapter::new(archive.storage.clone()),
    ))
    .unwrap()
    .with_form_responder(bridge);
    let supports_url = launcher.is_some();
    let executor = Arc::new(match launcher {
        Some(launcher) => executor.with_url_launcher(launcher),
        None => executor,
    });
    assert!(executor.execution_policy().form);
    assert_eq!(executor.execution_policy().url, supports_url);
    let prepare = move |runtime: &NativeMcpRuntime, writes| {
        scripted_candidate(
            runtime,
            writes,
            &json!({"name":"lookup","inputSchema":{"type":"object"}}),
            response,
        )
    };
    let fixture = match clock {
        Some(clock) => Fixture::with_executor_and_clock(
            arguments,
            PermissionMode::Auto,
            executor.clone(),
            executor.execution_policy(),
            false,
            clock,
            prepare,
        ),
        None => Fixture::with_executor(
            arguments,
            PermissionMode::Auto,
            executor.clone(),
            executor.execution_policy(),
            false,
            prepare,
        ),
    };
    (fixture, inbox)
}
fn envelope(id: i64, body: &str) -> Box<[u8]> {
    format!(r#"{{"jsonrpc":"2.0","id":{id},{body}}}"#)
        .into_bytes()
        .into()
}
fn answer(input: &str) -> NativeInteractivePromptResponse {
    NativeInteractivePromptResponse::Elicitation(
        McpElicitationAnswerInput::new(
            serde_json::value::RawValue::from_string(input.into()).unwrap(),
        )
        .unwrap(),
    )
}
fn run_answered(
    fixture: &Fixture,
    inbox: &mut NativeInteractivePromptInbox,
    reply: &str,
) -> (Vec<EngineEvent>, usize) {
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    let mut prompts = 0;
    let events = block_on(async {
        let responder = async {
            loop {
                let view = std::future::poll_fn(|cx| inbox.poll_prompt(cx))
                    .await
                    .expect("live inbox");
                let elicitation = view.elicitation().expect("form, not permission prompt");
                assert_eq!(elicitation.server(), "calendar");
                let crate::mcp::interaction::McpElicitationPromptSource::ModelTool {
                    context, ..
                } = elicitation.source()
                else {
                    panic!("actual model tool prompt source");
                };
                assert_eq!(context.call_id.as_str(), "call-0");
                prompts += 1;
                inbox.reply(view.token(), answer(reply)).unwrap();
            }
        };
        match select(Box::pin(turn.collect::<Vec<_>>()), Box::pin(responder)).await {
            Either::Left((events, _)) => events,
            Either::Right(_) => unreachable!(),
        }
    });
    (
        events
            .into_iter()
            .collect::<std::result::Result<_, _>>()
            .unwrap(),
        prompts,
    )
}
fn wires(fixture: &Fixture) -> Vec<Value> {
    fixture
        .writes
        .lock()
        .unwrap()
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| machine_god_core::json::from_slice(line).unwrap())
        .collect()
}

fn queued_prompt(
    future: &mut (impl std::future::Future + Unpin),
    inbox: &mut NativeInteractivePromptInbox,
) -> NativeInteractivePromptView {
    block_on(std::future::poll_fn(|cx| {
        // Real archive preparation owns a worker even for inline arguments.
        // Register actual wakeups rather than racing it with a busy poll loop.
        assert!(
            std::pin::Pin::new(&mut *future).poll(cx).is_pending(),
            "turn completed before displaying form"
        );
        match inbox.poll_prompt(cx) {
            Poll::Ready(Some(view)) => Poll::Ready(view),
            Poll::Ready(None) => panic!("inbox closed before form"),
            Poll::Pending => Poll::Pending,
        }
    }))
}

#[test]
fn accepted_form_uses_one_original_auto_grant_fresh_ids_and_exact_archive() {
    let archive = Archive::new();
    let args = machine_god_core::json::from_str(r#"{"exact":9007199254740993.0000001,"tiny":1e-99999,"zero":-0,"$serde_json::private::Number":"literal"}"#).unwrap();
    let raw = format!(
        r#"{{"resultType":"complete","content":[{{"type":"text","text":"{}"}}],"structuredContent":{{"n":1e400}}}}"#,
        "x".repeat(70_000)
    );
    let expected = ToolOutput::success(machine_god_core::json::from_str(&raw).unwrap());
    let (fixture, mut inbox) = configured(&archive, std::slice::from_ref(&args), move |id| {
        if id == 1 {
            envelope(id, INPUT)
        } else {
            envelope(id, &format!(r#""result":{raw}"#))
        }
    });
    let (events, prompts) = run_answered(
        &fixture,
        &mut inbox,
        r#"{"action":"accept","content":{"number":9007199254740993.0000001}}"#,
    );
    assert_eq!(prompts, 1);
    assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 1);
    assert_eq!(result(&events), &expected);
    let stored = persisted(&record(&fixture), "call-0");
    assert_eq!(archive.read(&stored.content), expected);
    let wire = wires(&fixture);
    assert_eq!(wire.len(), 2);
    assert_eq!(wire[0]["id"], 1);
    assert_eq!(wire[1]["id"], 2);
    assert_eq!(wire[0]["params"]["arguments"], args);
    assert_eq!(wire[1]["params"]["arguments"], args);
    assert!(wire[0]["params"].get("inputResponses").is_none());
    assert_eq!(
        wire[1]["params"]["inputResponses"]["confirm"]["action"],
        "accept"
    );
    assert_eq!(
        serde_json::to_string(&wire[1]["params"]["inputResponses"]["confirm"]["content"]["number"])
            .unwrap(),
        "9007199254740993.0000001"
    );
    assert_eq!(
        serde_json::to_string(&wire[1]["params"]["requestState"]["exact"]).unwrap(),
        "1e-99999"
    );
    assert_ne!(wire[0]["params"]["_meta"], wire[1]["params"]["_meta"]);
    assert_eq!(fixture.provider.requests().len(), 3);
}

#[test]
fn decline_and_cancel_are_forwarded_explicitly_not_inferred_acceptance() {
    for action in ["decline", "cancel"] {
        let archive = Archive::new();
        let (fixture, mut inbox) = configured(&archive, &[json!({})], |id| {
            envelope(
                id,
                if id == 1 {
                    INPUT
                } else {
                    r#""result":{"resultType":"complete","content":[],"isError":true}"#
                },
            )
        });
        let (events, prompts) =
            run_answered(&fixture, &mut inbox, &format!(r#"{{"action":"{action}"}}"#));
        assert_eq!(prompts, 1);
        assert!(result(&events).is_error);
        assert_eq!(
            wires(&fixture)[1]["params"]["inputResponses"]["confirm"],
            json!({"action":action})
        );
        assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn continuation_limit_is_initial_plus_eight_without_a_ninth_prompt_or_reservation() {
    let archive = Archive::new();
    let (fixture, mut inbox) = configured(&archive, &[json!({})], |id| envelope(id, INPUT));
    let (events, prompts) =
        run_answered(&fixture, &mut inbox, r#"{"action":"accept","content":{}}"#);
    assert_eq!(prompts, 8);
    assert_eq!(wires(&fixture).len(), 9);
    assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 1);
    assert_eq!(result(&events).content["resultType"], "protocol_failure");
    assert!(result(&events).is_error);
}

#[test]
fn state_only_empty_url_and_sampling_remain_explicit_unresolved_input_without_prompt() {
    for body in [
        r#""result":{"resultType":"input_required","requestState":null}"#,
        r#""result":{"resultType":"input_required","inputRequests":{}}"#,
        r#""result":{"resultType":"input_required","inputRequests":{"url":{"method":"elicitation/create","params":{"mode":"url","message":"Authorize","url":"https://example.test/auth"}}}}"#,
        r#""result":{"resultType":"input_required","inputRequests":{"roots":{"method":"roots/list"}}}"#,
    ] {
        let archive = Archive::new();
        let (fixture, mut inbox) = configured(&archive, &[json!({})], move |id| envelope(id, body));
        let events = fixture.run();
        assert_eq!(result(&events).content["resultType"], "input_required");
        assert_eq!(fixture.provider.requests().len(), 2);
        assert_eq!(wires(&fixture).len(), 1);
        assert!(
            inbox
                .poll_prompt(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
}

#[test]
fn cancellation_while_real_inbox_waits_revokes_consent_and_never_replays() {
    let archive = Archive::new();
    let (fixture, mut inbox) = configured(&archive, &[json!({})], |id| envelope(id, INPUT));
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    let handle = turn.handle().unwrap();
    let mut collect = Box::pin(turn.collect::<Vec<_>>());
    let view = queued_prompt(&mut collect, &mut inbox);
    assert!(handle.cancel());
    let events = block_on(collect)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(matches!(
        events.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert!(
        inbox
            .reply(view.token(), answer(r#"{"action":"accept","content":{}}"#))
            .is_err()
    );
    assert_eq!(wires(&fixture).len(), 1);
    assert_eq!(archive.names(), 0);
}

#[test]
fn retired_runtime_wakes_waiting_consent_and_discards_the_displayed_token() {
    let archive = Archive::new();
    let (fixture, mut inbox) = configured(&archive, &[json!({})], |id| envelope(id, INPUT));
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    let mut collect = Box::pin(turn.collect::<Vec<_>>());
    let view = queued_prompt(&mut collect, &mut inbox);
    fixture.runtime.close();
    let events = block_on(collect)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(result(&events).is_error);
    assert!(
        inbox
            .reply(view.token(), answer(r#"{"action":"accept","content":{}}"#))
            .is_err()
    );
    assert_eq!(wires(&fixture).len(), 1);
}

#[test]
fn original_permission_revocation_wakes_consent_without_waiting_for_an_answer() {
    let archive = Archive::new();
    let (fixture, mut inbox) = configured(&archive, &[json!({})], |id| envelope(id, INPUT));
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    let mut collect = Box::pin(turn.collect::<Vec<_>>());
    let view = queued_prompt(&mut collect, &mut inbox);
    fixture.conversation.permissions().unwrap().reset().unwrap();
    let events = block_on(collect)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(result(&events).is_error);
    assert!(
        inbox
            .reply(view.token(), answer(r#"{"action":"accept","content":{}}"#))
            .is_err()
    );
    assert_eq!(wires(&fixture).len(), 1);
    assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 1);
}

#[test]
fn one_form_cancel_cancels_the_remaining_requests_without_more_prompts() {
    let archive = Archive::new();
    let body = r#""result":{"resultType":"input_required","inputRequests":{"first":{"method":"elicitation/create","params":{"message":"First","requestedSchema":{"type":"object","properties":{}}}},"second":{"method":"elicitation/create","params":{"message":"Second","requestedSchema":{"type":"object","properties":{}}}}}}"#;
    let (fixture, mut inbox) = configured(&archive, &[json!({})], move |id| {
        envelope(
            id,
            if id == 1 {
                body
            } else {
                r#""result":{"resultType":"complete","content":[]}"#
            },
        )
    });
    let (_, prompts) = run_answered(&fixture, &mut inbox, r#"{"action":"cancel"}"#);
    assert_eq!(prompts, 1);
    let wire = wires(&fixture);
    assert_eq!(
        wire[1]["params"]["inputResponses"],
        json!({"first":{"action":"cancel"},"second":{"action":"cancel"}})
    );
}

#[test]
fn malformed_or_wrong_id_response_never_enters_human_consent_or_replays() {
    for response in [
        r#"{"jsonrpc":"2.0","id":100,"result":{"resultType":"input_required","requestState":null}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"input_required"}}"#,
    ] {
        let archive = Archive::new();
        let (fixture, mut inbox) =
            configured(&archive, &[json!({})], move |_| response.as_bytes().into());
        let events = fixture.run();
        assert!(result(&events).is_error);
        assert!(result(&events).content.get("resultType").is_none());
        assert_eq!(wires(&fixture).len(), 1);
        assert!(
            inbox
                .poll_prompt(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
}

struct AdvancingClock {
    origin: Instant,
    elapsed: std::sync::atomic::AtomicU64,
    reads: std::sync::atomic::AtomicU64,
    waiter: futures_util::task::AtomicWaker,
}
impl AdvancingClock {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            elapsed: std::sync::atomic::AtomicU64::new(0),
            reads: std::sync::atomic::AtomicU64::new(0),
            waiter: futures_util::task::AtomicWaker::new(),
        }
    }
    fn advance(&self, seconds: u64) {
        self.elapsed.fetch_add(seconds, Ordering::SeqCst);
        self.waiter.wake();
    }
}
impl NativeMcpRuntimeClock for AdvancingClock {
    fn now(&self) -> Instant {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.origin + Duration::from_secs(self.elapsed.load(Ordering::SeqCst))
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::poll_fn(move |cx| {
            self.waiter.register(cx.waker());
            if self.now() >= deadline {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }))
    }
}

#[test]
fn input_without_a_responder_does_not_start_an_interaction_clock() {
    let post_response_reads = [
        (
            r#""result":{"resultType":"complete","content":[]}"#,
            "complete",
        ),
        (INPUT, "input_required"),
    ]
    .map(|(body, result_type)| {
        let archive = Archive::new();
        let clock = Arc::new(AdvancingClock::new());
        let observed = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let response_clock = clock.clone();
        let response_observed = observed.clone();
        let fixture = Fixture::with_executor_and_clock(
            &[json!({})],
            PermissionMode::Auto,
            archive.executor.clone(),
            archive.executor.execution_policy(),
            false,
            clock.clone(),
            move |runtime, writes| {
                scripted_candidate(
                    runtime,
                    writes,
                    &json!({"name":"lookup","inputSchema":{"type":"object"}}),
                    move |id| {
                        response_observed.store(
                            response_clock.reads.load(Ordering::SeqCst),
                            Ordering::SeqCst,
                        );
                        envelope(id, body)
                    },
                )
            },
        );
        let events = fixture.run();
        assert_eq!(result(&events).content["resultType"], result_type);
        assert_eq!(wires(&fixture).len(), 1);
        let before = observed.load(Ordering::SeqCst);
        assert!(before > 0);
        clock
            .reads
            .load(Ordering::SeqCst)
            .checked_sub(before)
            .unwrap()
    });
    // Both exchanges must settle queued notifications under their operation
    // deadline after receiving a response. Unresolved input without a presenter
    // must add no interaction-clock work beyond that ordinary completion path.
    assert_eq!(post_response_reads[1], post_response_reads[0]);
}

#[test]
fn consent_before_expiry_allows_response_after_old_human_deadline() {
    let archive = Archive::new();
    let clock = Arc::new(AdvancingClock::new());
    let response_clock = clock.clone();
    let (fixture, mut inbox) = configured_with_clock(
        &archive,
        &[json!({})],
        move |id| {
            if id > 1 {
                // Response arrives after the human deadline, but only two
                // seconds into the fresh 120-second operation budget.
                response_clock.advance(2);
            }
            envelope(
                id,
                if id == 1 {
                    INPUT
                } else {
                    r#""result":{"resultType":"complete","content":[]}"#
                },
            )
        },
        Some(clock.clone()),
    );
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    let mut collect = Box::pin(turn.collect::<Vec<_>>());
    let view = queued_prompt(&mut collect, &mut inbox);
    clock.advance(1799); // One second before this round's human expiry.
    inbox
        .reply(view.token(), answer(r#"{"action":"accept","content":{}}"#))
        .unwrap();
    let events = block_on(collect)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(!result(&events).is_error);
    assert_eq!(clock.elapsed.load(Ordering::SeqCst), 1801);
    assert_eq!(wires(&fixture).len(), 2);
    assert_eq!(fixture.transport.reviews.load(Ordering::SeqCst), 1);
}

#[test]
fn each_round_gets_a_fresh_interaction_deadline_and_expiry_wakes_without_an_answer() {
    let archive = Archive::new();
    let clock = Arc::new(AdvancingClock::new());
    let (fixture, mut inbox) = configured_with_clock(
        &archive,
        &[json!({})],
        |id| envelope(id, INPUT),
        Some(clock.clone()),
    );
    fixture.conversation.enqueue("go".into()).unwrap();
    let turn = block_on(fixture.conversation.start_next(1))
        .unwrap()
        .unwrap();
    let mut collect = Box::pin(turn.collect::<Vec<_>>());
    let first = queued_prompt(&mut collect, &mut inbox);
    clock.advance(1000);
    inbox
        .reply(first.token(), answer(r#"{"action":"accept","content":{}}"#))
        .unwrap();
    let second = queued_prompt(&mut collect, &mut inbox);
    clock.advance(1000);
    // Total human time exceeds 30 minutes, but this round is still current.
    inbox
        .reply(
            second.token(),
            answer(r#"{"action":"accept","content":{}}"#),
        )
        .unwrap();
    let third = queued_prompt(&mut collect, &mut inbox);
    clock.advance(1800);
    let events = block_on(collect)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(result(&events).is_error);
    assert_eq!(wires(&fixture).len(), 3);
    assert!(
        inbox
            .reply(third.token(), answer(r#"{"action":"accept","content":{}}"#))
            .is_err()
    );
}
