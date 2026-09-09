use std::collections::VecDeque;
use std::future::{Future, pending};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures_util::{Stream, task::noop_waker};
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, Message, ProviderError, ProviderErrorKind, Role,
    SessionId, ToolCall, ToolCallId, ToolName,
};
use machine_god_native::{
    AiGatewayByteStream, AiGatewayPermissionReviewer, AiGatewayTransport,
    AiGatewayTransportRequest, NativeAutoPermissionAction as Action,
    NativeAutoPermissionAssessment as Assessment,
    NativeAutoPermissionAuthorization as Authorization, NativeAutoPermissionDecision as Decision,
    NativeAutoPermissionFilePreimage as Preimage, NativeAutoPermissionOrigin as Origin,
    NativeAutoPermissionPhase as Phase, NativeAutoPermissionReview as Review,
    NativeAutoPermissionReviewError as Error, NativeAutoPermissionRisk as Risk,
    NativeAutoPermissionRootContext as RootContext, NativeAutoPermissionSandboxScope as Scope,
    NativeAutoPermissionTarget as Target, NativePermissionReviewClock, NativePermissionReviewer,
};
use serde_json::{Value, json};
use sha2::Digest as _;

struct Clock {
    start: Instant,
    millis: AtomicU64,
    timers: AtomicUsize,
    timer_drops: Arc<AtomicUsize>,
}
impl Clock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            start: Instant::now(),
            millis: AtomicU64::new(0),
            timers: AtomicUsize::new(0),
            timer_drops: Arc::default(),
        })
    }
}
struct Timer {
    drops: Arc<AtomicUsize>,
}
impl Future for Timer {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl NativePermissionReviewClock for Clock {
    fn now(&self) -> Instant {
        self.start + Duration::from_millis(self.millis.load(Ordering::SeqCst))
    }
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'static, ()> {
        assert_eq!(deadline, self.now() + Duration::from_secs(15));
        self.timers.fetch_add(1, Ordering::SeqCst);
        Box::pin(Timer {
            drops: self.timer_drops.clone(),
        })
    }
}
#[derive(Default)]
struct Counts {
    constructs: AtomicUsize,
    polls: AtomicUsize,
    startup_drops: AtomicUsize,
    stream_drops: AtomicUsize,
}
struct Guard(Arc<Counts>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.startup_drops.fetch_add(1, Ordering::SeqCst);
    }
}
type RecordedWire = (Vec<(String, String)>, Value);
// Independent fixture fault switches intentionally exercise their combinations.
#[allow(clippy::struct_excessive_bools)]
struct Transport {
    wire: Mutex<Vec<RecordedWire>>,
    chunks: Mutex<VecDeque<Vec<u8>>>,
    counts: Arc<Counts>,
    clock: Arc<Clock>,
    startup_pending: bool,
    stream_pending: bool,
    error: Option<(ProviderErrorKind, bool)>,
    advance_startup: bool,
    advance_stream: bool,
    cancel_startup: bool,
    cancel_stream: bool,
}
impl Transport {
    fn new(bytes: Vec<u8>, clock: Arc<Clock>) -> Self {
        Self {
            wire: Mutex::default(),
            chunks: Mutex::new(VecDeque::from([bytes])),
            counts: Arc::default(),
            clock,
            startup_pending: false,
            stream_pending: false,
            error: None,
            advance_startup: false,
            advance_stream: false,
            cancel_startup: false,
            cancel_stream: false,
        }
    }
}
impl AiGatewayTransport for Transport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        self.counts.constructs.fetch_add(1, Ordering::SeqCst);
        assert!(request.body().len() <= 16 * 1024);
        self.wire.lock().unwrap().push((
            request
                .headers()
                .iter()
                .map(|h| (h.name().to_owned(), h.value().to_owned()))
                .collect(),
            serde_json::from_slice(request.body()).unwrap(),
        ));
        let guard = Guard(self.counts.clone());
        Box::pin(async move {
            let _guard = guard;
            self.counts.polls.fetch_add(1, Ordering::SeqCst);
            if self.advance_startup {
                self.clock.millis.store(15_000, Ordering::SeqCst);
            }
            if self.cancel_startup {
                cancellation.cancel();
            }
            if self.startup_pending {
                return pending().await;
            }
            if let Some((kind, retryable)) = self.error {
                return Err(ProviderError::new(
                    kind,
                    "private",
                    "private provider secret",
                    retryable,
                ));
            }
            Ok(Box::pin(Bytes {
                chunks: std::mem::take(&mut *self.chunks.lock().unwrap()),
                counts: self.counts.clone(),
                clock: self.clock.clone(),
                pending: self.stream_pending,
                advance: self.advance_stream,
                cancel: self.cancel_stream,
                cancellation,
            }) as AiGatewayByteStream)
        })
    }
}
struct Bytes {
    chunks: VecDeque<Vec<u8>>,
    counts: Arc<Counts>,
    clock: Arc<Clock>,
    pending: bool,
    advance: bool,
    cancel: bool,
    cancellation: CancellationToken,
}
impl Drop for Bytes {
    fn drop(&mut self) {
        self.counts.stream_drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl Stream for Bytes {
    type Item = Result<Vec<u8>, ProviderError>;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.advance {
            self.clock.millis.store(15_000, Ordering::SeqCst);
        }
        if self.cancel {
            self.cancellation.cancel();
        }
        if self.pending {
            Poll::Pending
        } else {
            Poll::Ready(self.chunks.pop_front().map(Ok))
        }
    }
}
struct Fixture {
    session: SessionId,
    call_id: ToolCallId,
    message: Message,
}
impl Fixture {
    fn new() -> Self {
        let call_id = ToolCallId::new("pending-1").unwrap();
        let message = Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "UNTRUSTED_ASSISTANT_PROSE".into(),
                },
                ContentBlock::Json {
                    value: json!({"image":"UNTRUSTED_IMAGE"}),
                },
                ContentBlock::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("sibling").unwrap(),
                        name: ToolName::new("delete_file").unwrap(),
                        arguments: json!({"path":"SIBLING_SECRET_PATH"}),
                    },
                },
                ContentBlock::ToolCall {
                    call: ToolCall {
                        id: call_id.clone(),
                        name: ToolName::new("terminal").unwrap(),
                        arguments: json!({"command":"cargo test"}),
                    },
                },
            ],
        };
        Self {
            session: SessionId::new("session").unwrap(),
            call_id,
            message,
        }
    }
    fn review(&self) -> Review<'_> {
        Review {
            session_id: &self.session,
            workspace_root: "/workspace",
            source_model: "source/model",
            pending_assistant: &self.message,
            target_call_id: &self.call_id,
            trusted_root_context: RootContext::from_proven_projection(
                "current_request: Implement and test the feature.\n",
            )
            .unwrap(),
            origin: Origin::Root,
            phase: Phase::Initial,
            targets: &[],
            action: Action::Command {
                command: "cargo test",
                resolved_cwd: "/workspace",
                background: false,
                backend: "none",
                target_os: "linux",
                scope: Scope::Restricted,
            },
            escalation_reason: "tool_requires_approval",
        }
    }
}
fn event(value: &Value) -> String {
    format!("data: {value}\n\n")
}
fn finish() -> String {
    event(&json!({"type":"finish","finishReason":{"unified":"tool-calls"}}))
}
fn arguments() -> Value {
    json!({"risk":"critical","authorization":"unknown","decision":"allow","rationale":"Requested development task."})
}
fn completion(args: &Value) -> Vec<u8> {
    (event(&json!({"type":"tool-call","toolCallId":"decision","toolName":"permission_decision","input":args}))+&finish()).into_bytes()
}
fn poll<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(&noop_waker()))
}

#[test]
fn actual_wire_pins_model_no_controls_and_only_exact_call_and_trusted_context() {
    futures_executor::block_on(async {
        let clock = Clock::new();
        let transport = Arc::new(Transport::new(completion(&arguments()), clock.clone()));
        let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock.clone());
        let fixture = Fixture::new();
        let result = reviewer
            .review(fixture.review(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.decision(), Decision::Allow);
        assert_eq!(result.risk(), Risk::Critical);
        assert_eq!(result.authorization(), Authorization::Unknown);
        let wire = transport.wire.lock().unwrap();
        assert_eq!(wire.len(), 1);
        assert!(
            wire[0]
                .0
                .contains(&("ai-language-model-id".into(), "zai/glm-5.2".into()))
        );
        let body = &wire[0].1;
        assert!(body.get("providerOptions").is_none());
        assert!(body.get("reasoningEffort").is_none());
        assert_eq!(body["toolChoice"], json!({"type":"required"}));
        assert_eq!(body["maxOutputTokens"], 2048);
        assert_eq!(body["prompt"].as_array().unwrap().len(), 4);
        assert_eq!(body["prompt"][1]["content"].as_array().unwrap().len(), 1);
        assert_eq!(body["prompt"][1]["content"][0]["toolCallId"], "pending-1");
        assert_eq!(
            body["prompt"][1]["content"][0]["input"],
            json!({"command":"cargo test"})
        );
        let encoded = body.to_string();
        let instruction = body["prompt"][3]["content"].as_str().unwrap();
        let marker = "<review_data encoding=\"xml-escaped-text\">";
        let start = instruction.find(marker).unwrap() + marker.len();
        let end = start + instruction[start..].find("</review_data>").unwrap();
        let original_policy = format!(
            "{}{{{{REVIEW_DATA}}}}{}",
            &instruction[..start],
            &instruction[end..]
        );
        assert_eq!(original_policy.len(), 5003);
        assert_eq!(
            format!("{:x}", sha2::Sha256::digest(original_policy.as_bytes())),
            "93c668d07bf4ab6974b451479005c05ae6c4afb6f8c7433d30035ae03087da38"
        );
        for omitted in [
            "UNTRUSTED_ASSISTANT_PROSE",
            "UNTRUSTED_IMAGE",
            "SIBLING_SECRET_PATH",
            "source/model",
        ] {
            assert!(!encoded.contains(omitted));
        }
        assert_eq!(transport.counts.constructs.load(Ordering::SeqCst), 1);
        assert_eq!(transport.counts.stream_drops.load(Ordering::SeqCst), 1);
        assert_eq!(clock.timer_drops.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn fragmented_sse_and_fragmented_strict_tool_input_are_accepted() {
    futures_executor::block_on(async {
        let args = arguments().to_string();
        let mut response = event(
            &json!({"type":"tool-input-start","id":"decision","toolName":"permission_decision"}),
        );
        for part in args.as_bytes().chunks(3) {
            response += &event(
                &json!({"type":"tool-input-delta","id":"decision","delta":std::str::from_utf8(part).unwrap()}),
            );
        }
        response += &event(&json!({"type":"tool-input-end","id":"decision"}));
        response += &event(
            &json!({"type":"tool-call","toolCallId":"decision","toolName":"permission_decision"}),
        );
        response += &finish();
        let clock = Clock::new();
        let transport = Arc::new(Transport::new(vec![], clock.clone()));
        *transport.chunks.lock().unwrap() = response.bytes().map(|byte| vec![byte]).collect();
        let reviewer = AiGatewayPermissionReviewer::new(transport, clock);
        let fixture = Fixture::new();
        assert_eq!(
            reviewer
                .review(fixture.review(), CancellationToken::new())
                .await
                .unwrap()
                .decision(),
            Decision::Allow
        );
    });
}

#[test]
fn invalid_output_never_retries_or_manufactures_a_decision() {
    futures_executor::block_on(async {
        let mut outputs = vec![];
        for args in [
            json!({}),
            json!({"risk":"low","authorization":"high","decision":"deny","rationale":"No"}),
            json!({"risk":"low","authorization":"high","decision":"allow","rationale":"","confidence":1}),
            json!({"risk":"low","authorization":"high","decision":"allow","rationale":"x".repeat(241)}),
        ] {
            outputs.push(completion(&args));
        }
        outputs.push(
            (event(&json!({"type":"text-delta","id":"text","delta":"Mixed prose"}))
                + std::str::from_utf8(&completion(&arguments())).unwrap())
            .into_bytes(),
        );
        outputs.push(
            (event(
                &json!({"type":"tool-call","toolCallId":"a","toolName":"wrong","input":arguments()}),
            ) + &finish())
                .into_bytes(),
        );
        outputs.push((event(&json!({"type":"tool-call","toolCallId":"a","toolName":"permission_decision","input":arguments()}))+
        &event(&json!({"type":"tool-call","toolCallId":"b","toolName":"permission_decision","input":arguments()}))+&finish()).into_bytes());
        outputs.push(b"data: {\"type\":\"tool-call\",\"toolCallId\":\"a\",\"toolName\":\"permission_decision\",\"input\":{\"risk\":\"low\",\"risk\":\"high\",\"authorization\":\"high\",\"decision\":\"allow\",\"rationale\":\"ok\"}}\n\n".to_vec());
        outputs.push(vec![]);
        for bytes in outputs {
            let clock = Clock::new();
            let transport = Arc::new(Transport::new(bytes, clock.clone()));
            let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock);
            let fixture = Fixture::new();
            assert_eq!(
                reviewer
                    .review(fixture.review(), CancellationToken::new())
                    .await,
                Err(Error::InvalidResponse)
            );
            assert_eq!(transport.counts.constructs.load(Ordering::SeqCst), 1);
        }
    });
}

#[test]
fn strict_assessment_checks_extra_fields_and_utf8_byte_rationale_bound() {
    futures_executor::block_on(async {
        assert!(
            Assessment::new(
                Risk::Low,
                Authorization::Unknown,
                Decision::Ask,
                &"é".repeat(120)
            )
            .is_ok()
        );
        assert_eq!(
            Assessment::new(
                Risk::Low,
                Authorization::Unknown,
                Decision::Ask,
                &"é".repeat(121)
            ),
            Err(Error::InvalidResponse)
        );
        let mut args = arguments();
        args["confidence"] = json!(1);
        let clock = Clock::new();
        let transport = Arc::new(Transport::new(completion(&args), clock.clone()));
        let reviewer = AiGatewayPermissionReviewer::new(transport, clock);
        let fixture = Fixture::new();
        assert_eq!(
            reviewer
                .review(fixture.review(), CancellationToken::new())
                .await,
            Err(Error::InvalidResponse)
        );
    });
}

#[test]
fn errors_remain_distinct_without_retry() {
    futures_executor::block_on(async {
        for (kind, retryable, expected) in [
            (
                ProviderErrorKind::Authentication,
                false,
                Error::PermanentFailure,
            ),
            (
                ProviderErrorKind::RateLimited,
                true,
                Error::TransientFailure,
            ),
            (ProviderErrorKind::Transport, false, Error::PermanentFailure),
            (ProviderErrorKind::Transport, true, Error::TransientFailure),
            (ProviderErrorKind::Protocol, false, Error::PermanentFailure),
            (ProviderErrorKind::Cancelled, false, Error::Cancelled),
        ] {
            let clock = Clock::new();
            let mut transport = Transport::new(vec![], clock.clone());
            transport.error = Some((kind, retryable));
            let transport = Arc::new(transport);
            let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock);
            let fixture = Fixture::new();
            assert_eq!(
                reviewer
                    .review(fixture.review(), CancellationToken::new())
                    .await,
                Err(expected)
            );
            assert_eq!(transport.counts.constructs.load(Ordering::SeqCst), 1);
        }
    });
}

#[test]
fn inert_before_poll_and_drop_release_pending_owned_startup_or_stream() {
    for startup in [true, false] {
        let clock = Clock::new();
        let mut transport = Transport::new(vec![], clock.clone());
        transport.startup_pending = startup;
        transport.stream_pending = !startup;
        let transport = Arc::new(transport);
        let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock.clone());
        let fixture = Fixture::new();
        let mut future = reviewer.review(fixture.review(), CancellationToken::new());
        assert_eq!(clock.timers.load(Ordering::SeqCst), 0);
        assert_eq!(transport.counts.constructs.load(Ordering::SeqCst), 0);
        assert!(poll(future.as_mut()).is_pending());
        assert_eq!(transport.counts.constructs.load(Ordering::SeqCst), 0);
        assert!(poll(future.as_mut()).is_pending());
        if !startup {
            assert!(poll(future.as_mut()).is_pending());
        }
        drop(future);
        assert_eq!(clock.timer_drops.load(Ordering::SeqCst), 1);
        assert_eq!(transport.counts.startup_drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            transport.counts.stream_drops.load(Ordering::SeqCst),
            usize::from(!startup)
        );
    }
}

#[test]
fn deadline_and_cancellation_win_during_startup_or_stream_same_poll() {
    for at_startup in [true, false] {
        for cancel in [true, false] {
            let clock = Clock::new();
            let mut transport = Transport::new(completion(&arguments()), clock.clone());
            transport.cancel_startup = at_startup && cancel;
            transport.cancel_stream = !at_startup && cancel;
            transport.advance_startup = at_startup && !cancel;
            transport.advance_stream = !at_startup && !cancel;
            let transport = Arc::new(transport);
            let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock);
            let fixture = Fixture::new();
            let mut future = reviewer.review(fixture.review(), CancellationToken::new());
            assert!(poll(future.as_mut()).is_pending());
            if !at_startup {
                assert!(poll(future.as_mut()).is_pending());
            }
            assert_eq!(
                poll(future.as_mut()),
                Poll::Ready(Err(if cancel {
                    Error::Cancelled
                } else {
                    Error::TimedOut
                }))
            );
            assert_eq!(transport.counts.startup_drops.load(Ordering::SeqCst), 1);
            assert_eq!(transport.counts.stream_drops.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn cancellation_and_clock_deadline_wake_boundaries_do_not_require_transport_readiness() {
    for cancel in [true, false] {
        let clock = Clock::new();
        let mut transport = Transport::new(vec![], clock.clone());
        transport.startup_pending = true;
        let transport = Arc::new(transport);
        let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock.clone());
        let fixture = Fixture::new();
        let token = CancellationToken::new();
        let mut future = reviewer.review(fixture.review(), token.clone());
        assert!(poll(future.as_mut()).is_pending());
        assert!(poll(future.as_mut()).is_pending());
        if cancel {
            token.cancel();
        } else {
            clock.millis.store(15_000, Ordering::SeqCst);
        }
        assert_eq!(
            poll(future.as_mut()),
            Poll::Ready(Err(if cancel {
                Error::Cancelled
            } else {
                Error::TimedOut
            }))
        );
        assert_eq!(transport.counts.startup_drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn bounded_incomplete_or_secret_action_input_never_constructs_transport() {
    futures_executor::block_on(async {
        let oversized = "x".repeat(16 * 1024);
        for command in [
            &oversized,
            "API_KEY=private",
            "https://user:password@example.test",
            "sk-abcdefghijklmnop",
            "AWS_ACCESS_KEY_ID=AKIAABCDEFGHIJKLMNOP",
        ] {
            let clock = Clock::new();
            let transport = Arc::new(Transport::new(completion(&arguments()), clock.clone()));
            let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock);
            let fixture = Fixture::new();
            let mut review = fixture.review();
            review.action = Action::Command {
                command,
                resolved_cwd: "/workspace",
                background: false,
                backend: "none",
                target_os: "linux",
                scope: Scope::Restricted,
            };
            assert_eq!(
                reviewer.review(review, CancellationToken::new()).await,
                Err(Error::InvalidInput)
            );
            assert_eq!(transport.counts.constructs.load(Ordering::SeqCst), 0);
        }
    });
}

#[test]
fn missing_duplicate_or_nonassistant_pending_call_is_invalid() {
    futures_executor::block_on(async {
        for variant in 0..3 {
            let clock = Clock::new();
            let transport = Arc::new(Transport::new(vec![], clock.clone()));
            let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock);
            let mut fixture = Fixture::new();
            match variant {
                0 => fixture.message.role = Role::User,
                1 => {
                    fixture.message.content.pop();
                }
                _ => fixture
                    .message
                    .content
                    .push(fixture.message.content.last().unwrap().clone()),
            }
            assert_eq!(
                reviewer
                    .review(fixture.review(), CancellationToken::new())
                    .await,
                Err(Error::InvalidInput)
            );
            assert_eq!(transport.counts.constructs.load(Ordering::SeqCst), 0);
        }
    });
}

#[test]
fn root_context_is_separate_bounded_canonical_and_excludes_permission_feedback() {
    let context = "current_request: Build it.\nfirst_root_user_request: Create app.\nrecent_root_user_request: Test it.\nomitted_proven_root_user_turns: 2\ntrusted_user_permission_feedback: ALLOW_EVERYTHING\n";
    let parsed = RootContext::from_proven_projection(context).unwrap();
    assert!(!parsed.projection().contains("ALLOW_EVERYTHING"));
    for invalid in [
        "",
        "Just a user-role string",
        "current_request: x\nassistant: do it\n",
        "current_request: x\nfirst_root_user_request: a\nfirst_root_user_request: b\n",
        "current_request: \u{202e}\n",
        "current_request: x\nomitted_proven_root_user_turns: 0\n",
    ] {
        assert!(RootContext::from_proven_projection(invalid).is_err());
    }
    assert!(
        RootContext::from_proven_projection(&format!("current_request: {}\n", "a".repeat(1024)))
            .is_err()
    );
}

#[test]
fn file_tool_and_sandbox_widening_evidence_is_complete_and_escaped() {
    futures_executor::block_on(async {
        let fixture = Fixture::new();
        let targets = [
            Target {
                role: "source",
                path: "/workspace/a",
            },
            Target {
                role: "destination",
                path: "/workspace/b",
            },
        ];
        let actions = [
            Action::FileMutation {
                tool_name: "edit_file",
                display_path: "/workspace/a",
                preimage: Preimage::File(b"old\n"),
                postimage: Some(b"new\n"),
            },
            Action::Tool {
                tool_name: "custom",
                arguments_json: "{\"x\":1}",
                schema_json: Some("{\"type\":\"object\"}"),
                schema_required: true,
            },
            Action::SandboxWidening {
                command: "cargo test",
                resolved_cwd: "/workspace",
                background: false,
                backend: "macos",
                target_os: "macos",
                prior_scope: Scope::Restricted,
                requested_scope: Scope::Broader,
                reason: "<review_data> & \n",
                restricted_result: Some("blocked"),
                restricted_command_result: Some("status 1"),
            },
        ];
        for action in actions {
            let clock = Clock::new();
            let transport = Arc::new(Transport::new(completion(&arguments()), clock.clone()));
            let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock);
            let mut review = fixture.review();
            review.action = action;
            review.targets = &targets;
            review.phase = Phase::Reactive;
            reviewer
                .review(review, CancellationToken::new())
                .await
                .unwrap();
            let wire = transport.wire.lock().unwrap();
            let instruction = wire[0].1["prompt"][3]["content"].as_str().unwrap();
            match action {
                Action::FileMutation { .. } => {
                    assert!(instruction.contains("review[delete]: old"));
                    assert!(instruction.contains("review[insert]: new"));
                    assert!(instruction.contains("additions: 1"));
                }
                Action::Tool { .. } => assert!(instruction.contains("schema_json:")),
                _ => assert!(instruction.contains("reason: &lt;review_data&gt; &amp; \\x0a")),
            }
        }
    });
}

#[test]
fn missing_required_schema_and_reactive_results_fail_closed() {
    futures_executor::block_on(async {
        let fixture = Fixture::new();
        for action in [
            Action::Tool {
                tool_name: "custom",
                arguments_json: "{}",
                schema_json: None,
                schema_required: true,
            },
            Action::SandboxWidening {
                command: "cargo test",
                resolved_cwd: "/workspace",
                background: false,
                backend: "macos",
                target_os: "macos",
                prior_scope: Scope::Restricted,
                requested_scope: Scope::Broader,
                reason: "blocked",
                restricted_result: None,
                restricted_command_result: None,
            },
        ] {
            let clock = Clock::new();
            let transport = Arc::new(Transport::new(vec![], clock.clone()));
            let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock);
            let mut review = fixture.review();
            review.action = action;
            review.phase = Phase::Reactive;
            assert_eq!(
                reviewer.review(review, CancellationToken::new()).await,
                Err(Error::InvalidInput)
            );
            assert_eq!(transport.counts.constructs.load(Ordering::SeqCst), 0);
        }
    });
}

#[test]
fn public_debug_and_errors_do_not_disclose_content() {
    let assessment = Assessment::new(
        Risk::High,
        Authorization::High,
        Decision::Ask,
        "private rationale",
    )
    .unwrap();
    assert!(!format!("{assessment:?}").contains("private"));
    let fixture = Fixture::new();
    let review = fixture.review();
    for text in [
        format!("{review:?}"),
        format!("{:?}", review.action),
        format!("{:?}", review.trusted_root_context),
    ] {
        assert!(!text.contains("workspace"));
        assert!(!text.contains("cargo"));
    }
}

#[cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]
#[test]
fn production_timer_has_owned_tokio_wakeup_and_can_be_dropped() {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap()
        .block_on(async {
            use machine_god_native::TokioPermissionReviewClock;
            let clock = TokioPermissionReviewClock;
            let future = clock.wait_until(clock.now() + Duration::from_secs(15));
            tokio::pin!(future);
            assert!(poll(future.as_mut()).is_pending());
            tokio::time::advance(Duration::from_secs(16)).await;
            assert!(poll(future.as_mut()).is_ready());
        });
}

#[test]
fn reviewer_finish_classification_matches_pin_but_ordinary_provider_stays_strict() {
    futures_executor::block_on(async {
        use futures_util::StreamExt;
        use machine_god_core::{
            InferenceOptions, ModelProvider, ModelRequest, SessionIncarnationId, TurnId,
        };
        use machine_god_native::AiGatewayProvider;
        for (reason, expected) in [
            ("stop", Ok(())),
            ("length", Ok(())),
            ("other", Ok(())),
            ("content-filter", Err(Error::PermanentFailure)),
            ("error", Err(Error::TransientFailure)),
        ] {
            let response=(event(&json!({"type":"tool-call","toolCallId":"decision","toolName":"permission_decision","input":arguments()}))+
            &event(&json!({"type":"finish","finishReason":{"unified":reason}}))).into_bytes();
            let clock = Clock::new();
            let transport = Arc::new(Transport::new(response.clone(), clock.clone()));
            let reviewer = AiGatewayPermissionReviewer::new(transport, clock.clone());
            let fixture = Fixture::new();
            assert_eq!(
                reviewer
                    .review(fixture.review(), CancellationToken::new())
                    .await
                    .map(|_| ()),
                expected
            );
            let ordinary =
                AiGatewayProvider::new("source/model", Arc::new(Transport::new(response, clock)))
                    .unwrap();
            let request = ModelRequest {
                session_id: fixture.session.clone(),
                session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
                turn_id: TurnId::new("turn").unwrap(),
                messages: vec![Message::text(Role::User, "Hello")],
                tools: vec![],
                options: InferenceOptions::default(),
            };
            let mut stream = ordinary
                .stream(request, CancellationToken::new())
                .await
                .unwrap();
            let mut failure = None;
            while let Some(event) = stream.next().await {
                if let Err(error) = event {
                    failure = Some(error.kind);
                }
            }
            assert_eq!(
                failure,
                Some(if reason == "error" {
                    ProviderErrorKind::Other
                } else {
                    ProviderErrorKind::Protocol
                })
            );
        }
    });
}

#[test]
fn newline_only_file_change_remains_visible_and_response_ask_is_not_an_error() {
    futures_executor::block_on(async {
        let clock = Clock::new();
        let mut args = arguments();
        args["decision"] = json!("ask");
        let transport = Arc::new(Transport::new(completion(&args), clock.clone()));
        let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock);
        let fixture = Fixture::new();
        let mut review = fixture.review();
        review.action = Action::FileMutation {
            tool_name: "edit_file",
            display_path: "a",
            preimage: Preimage::File(b"same\n"),
            postimage: Some(b"same"),
        };
        assert_eq!(
            reviewer
                .review(review, CancellationToken::new())
                .await
                .unwrap()
                .decision(),
            Decision::Ask
        );
        let wire = transport.wire.lock().unwrap();
        let instruction = wire[0].1["prompt"][3]["content"].as_str().unwrap();
        assert!(instruction.contains("preimage_final_newline: true"));
        assert!(instruction.contains("postimage_final_newline: false"));
        assert!(instruction.contains("preimage_bytes: 5"));
        assert!(instruction.contains("postimage_bytes: 4"));
    });
}

#[test]
fn encoded_packet_limit_includes_json_escaping_and_selected_arguments() {
    futures_executor::block_on(async {
        for use_arguments in [false, true] {
            let clock = Clock::new();
            let transport = Arc::new(Transport::new(vec![], clock.clone()));
            let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock);
            let mut fixture = Fixture::new();
            let value = "\\\"".repeat(4000);
            if use_arguments
                && let ContentBlock::ToolCall { call } = fixture.message.content.last_mut().unwrap()
            {
                call.arguments = json!({"value":value});
            }
            let mut review = fixture.review();
            if !use_arguments {
                review.escalation_reason = &value;
            }
            assert_eq!(
                reviewer.review(review, CancellationToken::new()).await,
                Err(Error::InvalidInput)
            );
            assert_eq!(transport.counts.constructs.load(Ordering::SeqCst), 0);
        }
    });
}

#[test]
fn already_cancelled_review_never_acquires_timer_or_transport() {
    let clock = Clock::new();
    let transport = Arc::new(Transport::new(vec![], clock.clone()));
    let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock.clone());
    let fixture = Fixture::new();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut future = reviewer.review(fixture.review(), cancellation);
    assert_eq!(poll(future.as_mut()), Poll::Ready(Err(Error::Cancelled)));
    assert_eq!(clock.timers.load(Ordering::SeqCst), 0);
    assert_eq!(transport.counts.constructs.load(Ordering::SeqCst), 0);
}

#[test]
fn raw_file_bytes_are_completely_escaped_without_losing_byte_identity() {
    futures_executor::block_on(async {
        let clock = Clock::new();
        let transport = Arc::new(Transport::new(completion(&arguments()), clock.clone()));
        let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock);
        let fixture = Fixture::new();
        let mut review = fixture.review();
        review.action = Action::FileMutation {
            tool_name: "write_file",
            display_path: "a",
            preimage: Preimage::Absent,
            postimage: Some(b"\xff\0\n\xc3\xa9"),
        };
        reviewer
            .review(review, CancellationToken::new())
            .await
            .unwrap();
        let wire = transport.wire.lock().unwrap();
        let instruction = wire[0].1["prompt"][3]["content"].as_str().unwrap();
        assert!(instruction.contains("review[insert]: \\xff\\x00"));
        assert!(instruction.contains("review[insert]: é"));
        assert!(instruction.contains("postimage_bytes: 5"));
        assert!(instruction.contains("additions: 2"));
    });
}

#[test]
fn deadline_at_preparation_boundary_prevents_transport_construction() {
    let clock = Clock::new();
    let transport = Arc::new(Transport::new(vec![], clock.clone()));
    let reviewer = AiGatewayPermissionReviewer::new(transport.clone(), clock.clone());
    let fixture = Fixture::new();
    let mut future = reviewer.review(fixture.review(), CancellationToken::new());
    assert!(poll(future.as_mut()).is_pending());
    clock.millis.store(15_000, Ordering::SeqCst);
    assert_eq!(poll(future.as_mut()), Poll::Ready(Err(Error::TimedOut)));
    assert_eq!(transport.counts.constructs.load(Ordering::SeqCst), 0);
}
