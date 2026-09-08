#![cfg(any(target_os = "linux", target_os = "macos"))]

use futures_core::Stream;
use futures_executor::block_on;
use futures_util::{StreamExt, task::noop_waker};
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ContentBlock, Engine, FilesystemAccess, Message,
    ModelEvent, PermissionAuthorization, PermissionDecision, PermissionError, PermissionGrantScope,
    PermissionHandler, PermissionInvocation, PermissionRequest, PreparedToolCall, Role, Session,
    SessionId, SessionIncarnationId, SessionRecord, SessionRevision, StopReason, Tool, ToolCall,
    ToolCallId, ToolContext, ToolError, ToolName, ToolOutput, ToolSpec,
};
use machine_god_native::{
    NATIVE_PERMISSION_CONTEXT_KEY, NATIVE_SESSION_METADATA_KEY, NativeConversation,
    NativePermissionContexts, NativePermissionReviewContext, NativeSessionMetadata,
    NativeSessionOrigin,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, SessionStoreScript,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll};

#[derive(Default)]
struct State {
    reviewer: Mutex<Option<Arc<machine_god_native::AiGatewayPermissionReviewer>>>,
    seen: Mutex<Vec<NativePermissionReviewContext>>,
    roots: Mutex<Vec<Option<String>>>,
    waiting: AtomicBool,
    pause: AtomicBool,
}
struct Policy {
    contexts: Arc<NativePermissionContexts>,
    state: Arc<State>,
}
impl PermissionHandler for Policy {
    fn authorize(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        panic!("invocation required")
    }
    fn authorize_invocation<'a>(
        &'a self,
        request: PermissionRequest,
        invocation: PermissionInvocation<'a>,
    ) -> BoxFuture<'a, Result<PermissionAuthorization, PermissionError>> {
        Box::pin(async move {
            assert_eq!(invocation.arguments, &json!({"prepared":true}));
            let snapshot = self.contexts.snapshot(&request).unwrap();
            assert!(snapshot.is_live());
            assert_eq!(snapshot.target_call_id().as_str(), "reused");
            let [ContentBlock::ToolCall { call }] = snapshot.pending_assistant().content.as_slice()
            else {
                panic!("only current call")
            };
            assert_eq!(call.arguments, json!({"original":true}));
            let mut foreign = request.clone();
            foreign.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
            assert!(self.contexts.snapshot(&foreign).is_err());
            let reviewer = self.state.reviewer.lock().unwrap().clone();
            if let Some(reviewer) = reviewer {
                use machine_god_native::{
                    NativeAutoPermissionAction, NativeAutoPermissionOrigin,
                    NativeAutoPermissionPhase, NativeAutoPermissionReview,
                    NativePermissionReviewer,
                };
                let arguments = serde_json::to_string(invocation.arguments).unwrap();
                let result = reviewer
                    .review(
                        NativeAutoPermissionReview {
                            session_id: &request.session_id,
                            workspace_root: "/workspace",
                            source_model: snapshot.source_model().unwrap(),
                            pending_assistant: snapshot.pending_assistant(),
                            target_call_id: snapshot.target_call_id(),
                            trusted_root_context: snapshot.trusted_root_context().unwrap(),
                            origin: NativeAutoPermissionOrigin::Root,
                            phase: NativeAutoPermissionPhase::Initial,
                            targets: &[],
                            action: NativeAutoPermissionAction::Tool {
                                tool_name: "fixture",
                                arguments_json: &arguments,
                                schema_json: None,
                                schema_required: false,
                            },
                            escalation_reason: "",
                        },
                        CancellationToken::new(),
                    )
                    .await;
                assert!(result.is_err()); // Capture-only transport deliberately refuses completion.
            }
            self.state.roots.lock().unwrap().push(
                snapshot
                    .trusted_root_context()
                    .ok()
                    .map(|root| root.projection().to_owned()),
            );
            self.state.seen.lock().unwrap().push(snapshot);
            std::future::poll_fn(|_| {
                self.state.waiting.store(true, Ordering::SeqCst);
                if self.state.pause.load(Ordering::SeqCst) {
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
            Ok(PermissionAuthorization::new(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            }))
        })
    }
}
struct Fixture;
impl Tool for Fixture {
    fn complete_input_limits(&self) -> Option<machine_god_core::ToolInputLimits> {
        let limit = std::num::NonZeroUsize::new(1024).unwrap();
        Some(machine_god_core::ToolInputLimits {
            max_argument_bytes: limit,
            max_argument_nodes: limit,
            max_prepared_argument_bytes: limit,
            max_prepared_argument_nodes: limit,
        })
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("fixture").unwrap(),
            description: "fixture".into(),
            input_schema: json!({}),
        }
    }
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        Ok(if call.arguments.get("skip").is_some() {
            PreparedToolCall::without_authority(json!({"prepared":true}))
        } else {
            PreparedToolCall::new(
                Capability::Filesystem {
                    access: FilesystemAccess::Write,
                    path: "file".into(),
                },
                json!({"prepared":true}),
            )
        })
    }
    fn persist_arguments<'a>(
        &'a self,
        _: ToolContext,
        _: &'a Value,
        _: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
        Box::pin(async { Ok(Some(json!({"archive":true}))) })
    }
    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async { Ok(ToolOutput::success("done")) })
    }
}
fn round(skip: bool) -> ModelProviderStep {
    let mut events = vec![ModelEvent::TextDelta {
        text: "untrusted assistant prose".into(),
    }];
    if skip {
        events.push(ModelEvent::ToolCall {
            call: ToolCall {
                id: ToolCallId::new("skipped").unwrap(),
                name: ToolName::new("fixture").unwrap(),
                arguments: json!({"skip":true}),
            },
        });
    }
    events.push(ModelEvent::ToolCall {
        call: ToolCall {
            id: ToolCallId::new("reused").unwrap(),
            name: ToolName::new("fixture").unwrap(),
            arguments: json!({"original":true}),
        },
    });
    events.push(ModelEvent::Stop {
        reason: StopReason::ToolCalls,
    });
    ModelProviderStep::events(events)
}
fn done() -> ModelProviderStep {
    ModelProviderStep::events([ModelEvent::Stop {
        reason: StopReason::Completed,
    }])
}
fn setup(
    messages: Vec<Message>,
    steps: Vec<ModelProviderStep>,
) -> (
    Engine,
    NativeConversation,
    Session,
    Arc<State>,
    Arc<NativePermissionContexts>,
) {
    let mut record = SessionRecord::empty(
        SessionId::new("context").unwrap(),
        SessionIncarnationId::new("life").unwrap(),
    );
    record.revision = SessionRevision(1);
    record.messages = messages;
    record.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.into(),
        NativeSessionMetadata::new(
            std::path::Path::new("/workspace"),
            0,
            NativeSessionOrigin::Cli,
        )
        .unwrap()
        .to_value(),
    );
    let id = record.id.clone();
    let store = InMemorySessionStore::configured(
        BTreeMap::from([(id.clone(), record)]),
        SessionStoreScript::default(),
        100,
    );
    let contexts = Arc::new(NativePermissionContexts::new());
    let state = Arc::new(State::default());
    let engine = Engine::builder()
        .session_store(store)
        .provider(ScriptedModelProvider::new("context", steps))
        .permission_handler(Policy {
            contexts: contexts.clone(),
            state: state.clone(),
        })
        .tool(Fixture)
        .build()
        .unwrap();
    let session = block_on(engine.load_session(id)).unwrap().unwrap();
    let owner = NativeConversation::from_session(session.clone())
        .unwrap()
        .with_permission_contexts(&contexts)
        .unwrap();
    (engine, owner, session, state, contexts)
}
fn complete(owner: &NativeConversation, text: &str) {
    let turn = block_on(owner.prompt(text.into(), 1)).unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    assert!(
        events.iter().any(|event| matches!(
            &event.as_ref().unwrap().payload,
            machine_god_core::TurnEvent::PermissionRequested { .. }
        )),
        "{events:?}"
    );
}

#[test]
fn exact_current_call_has_original_arguments_and_canonical_cursor_with_reused_ids() {
    let (_engine, owner, session, state, _) =
        setup(vec![], vec![round(true), done(), round(false), done()]);
    complete(&owner, "actual root");
    complete(&owner, "actual root");
    let seen = state.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].source_cursor(), (1, 2));
    assert_eq!(seen[1].source_cursor(), (6, 1));
    for snapshot in seen.iter() {
        assert!(!snapshot.is_live());
        assert!(snapshot.trusted_root_context().is_err());
        let (message, block) = snapshot.source_cursor();
        let record = session.record_snapshot();
        let ContentBlock::ToolCall { call } = &record.messages[message].content[block] else {
            panic!("canonical call")
        };
        assert_eq!(call.arguments, json!({"archive":true}));
        assert!(!format!("{snapshot:?}").contains("actual root"));
    }
    assert_eq!(
        state.roots.lock().unwrap().as_slice(),
        [
            Some("current_request: actual root\n".into()),
            Some("current_request: actual root\nfirst_root_user_request: actual root\n".into())
        ]
    );
}

#[test]
fn root_provenance_excludes_preloaded_roles_and_tracks_only_native_admissions() {
    let messages = vec![
        Message::text(Role::System, "system"),
        Message::text(Role::User, "foreign role user"),
        Message::text(Role::Assistant, "foreign answer"),
    ];
    let (_engine, owner, _, state, _) =
        setup(messages, vec![round(false), done(), round(false), done()]);
    complete(&owner, "first actual");
    complete(&owner, "second actual");
    let roots = state.roots.lock().unwrap();
    assert_eq!(roots[0].as_deref(), Some("current_request: first actual\n"));
    assert_eq!(
        roots[1].as_deref(),
        Some("current_request: second actual\nrecent_root_user_request: first actual\n")
    );
    assert!(
        owner
            .record()
            .metadata
            .contains_key(NATIVE_PERMISSION_CONTEXT_KEY)
    );
}

#[test]
fn cancellation_and_drop_retire_retained_context_without_keeping_turn_alive() {
    for cancel in [false, true] {
        let (_engine, owner, session, state, _) = setup(vec![], vec![round(false), done()]);
        state.pause.store(true, Ordering::SeqCst);
        let mut turn = block_on(owner.prompt("root".into(), 1)).unwrap();
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        for _ in 0..64 {
            if state.waiting.load(Ordering::SeqCst) {
                break;
            }
            let _ = std::pin::Pin::new(&mut turn).poll_next(&mut cx);
        }
        assert!(state.waiting.load(Ordering::SeqCst));
        assert!(state.seen.lock().unwrap()[0].is_live());
        if cancel {
            let _ = turn.handle().cancel();
            assert!(!state.seen.lock().unwrap()[0].is_live());
        }
        drop(turn);
        assert!(!session.has_active_turn());
        assert!(!owner.is_busy());
        assert!(!state.seen.lock().unwrap()[0].is_live());
    }
}

#[test]
fn inert_prompt_does_not_publish_provenance_and_root_masking_is_bounded() {
    let (_engine, owner, _, state, _) = setup(vec![], vec![round(false), done()]);
    drop(owner.prompt("never admitted".into(), 1));
    assert!(
        !owner
            .record()
            .metadata
            .contains_key(NATIVE_PERMISSION_CONTEXT_KEY)
    );
    complete(
        &owner,
        &format!("head TOKEN=secret-value\n{} tail", "🙂".repeat(1000)),
    );
    let roots = state.roots.lock().unwrap();
    let root = roots[0].as_ref().unwrap();
    assert!(root.len() <= 1024);
    assert!(root.contains("[redacted]"));
    assert!(!root.contains("secret-value"));
    assert!(root.contains("\\x0a"));
    assert!(root.contains(" [... omitted ...] "));
    assert!(root.ends_with(" tail\n"));
}

#[test]
fn queued_cancellation_and_taken_model_are_not_later_selection() {
    use machine_god_native::{
        NativeConversationRuntime, NativeModelPreferences, NativeReasoningEffort,
    };
    let (_engine, owner, _, state, _) =
        setup(vec![], vec![round(false), done(), round(false), done()]);
    let prefs = |name: &str| {
        NativeModelPreferences::new(name, NativeReasoningEffort::parse("high").unwrap(), true)
            .unwrap()
    };
    let runtime = NativeConversationRuntime::new(owner, prefs("source/one"), None).unwrap();
    let cancelled = runtime.enqueue("discarded request".into()).unwrap();
    assert!(runtime.cancel_queued(cancelled));
    runtime.enqueue("taken root".into()).unwrap();
    let turn = block_on(runtime.start_next(1)).unwrap().unwrap();
    runtime.set_model_preferences(prefs("source/two")).unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    runtime.enqueue("next root".into()).unwrap();
    let turn = block_on(runtime.start_next(2)).unwrap().unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let seen = state.seen.lock().unwrap();
    assert_eq!(seen[0].source_model(), Some("source/one"));
    assert_eq!(seen[1].source_model(), Some("source/two"));
    assert_eq!(
        seen[0].model_snapshot().unwrap().preferences().model(),
        "source/one"
    );
    let roots = state.roots.lock().unwrap();
    assert_eq!(
        roots[1].as_deref(),
        Some("current_request: next root\nfirst_root_user_request: taken root\n")
    );
    assert!(
        !roots
            .iter()
            .flatten()
            .any(|root| root.contains("discarded"))
    );
}

#[test]
fn continuation_reuses_proven_root_and_resume_does_not_infer_missing_provenance() {
    for retain_provenance in [false, true] {
        let (_engine, owner, session, state, contexts) =
            setup(vec![], vec![round(false), round(false), done()]);
        state.pause.store(true, Ordering::SeqCst);
        let mut turn = block_on(owner.prompt("original root".into(), 1)).unwrap();
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        for _ in 0..64 {
            if state.waiting.load(Ordering::SeqCst) {
                break;
            }
            let _ = std::pin::Pin::new(&mut turn).poll_next(&mut cx);
        }
        assert!(state.waiting.load(Ordering::SeqCst));
        drop(turn);
        drop(owner);
        if !retain_provenance {
            let mut metadata = session.record().metadata;
            metadata.remove(NATIVE_PERMISSION_CONTEXT_KEY);
            let revision = session.record().revision;
            block_on(session.update_metadata(revision, metadata)).unwrap();
        }
        state.pause.store(false, Ordering::SeqCst);
        let owner = NativeConversation::from_session(session)
            .unwrap()
            .with_permission_contexts(&contexts)
            .unwrap();
        let turn = block_on(owner.continue_turn(machine_god_core::InferenceOptions::default(), 2))
            .unwrap();
        assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
        let roots = state.roots.lock().unwrap();
        assert_eq!(
            roots[1].as_deref(),
            retain_provenance.then_some("current_request: original root\n")
        );
    }
}

#[test]
fn real_reviewer_wire_uses_owned_root_and_exact_call_without_source_model_controls() {
    use machine_god_native::{
        AiGatewayByteStream, AiGatewayPermissionReviewer, AiGatewayTransport,
        AiGatewayTransportRequest, NativeConversationRuntime, NativeModelPreferences,
        NativePermissionReviewClock, NativeReasoningEffort,
    };
    struct Clock(std::time::Instant);
    impl NativePermissionReviewClock for Clock {
        fn now(&self) -> std::time::Instant {
            self.0
        }
        fn wait_until(&self, _: std::time::Instant) -> BoxFuture<'static, ()> {
            Box::pin(std::future::pending())
        }
    }
    #[derive(Default)]
    struct Capture(Mutex<Vec<Value>>);
    impl AiGatewayTransport for Capture {
        fn stream(
            &self,
            request: AiGatewayTransportRequest,
            _: CancellationToken,
        ) -> BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>> {
            assert!(
                request
                    .headers()
                    .iter()
                    .any(|header| header.name() == "ai-language-model-id"
                        && header.value() == "zai/glm-5.2")
            );
            self.0
                .lock()
                .unwrap()
                .push(serde_json::from_slice(request.body()).unwrap());
            Box::pin(async {
                Err(machine_god_core::ProviderError::new(
                    machine_god_core::ProviderErrorKind::Transport,
                    "fixture",
                    "fixture",
                    false,
                ))
            })
        }
    }
    let (_engine, owner, _, state, _) = setup(
        vec![Message::text(Role::User, "unproven user role")],
        vec![round(true), done()],
    );
    let capture = Arc::new(Capture::default());
    *state.reviewer.lock().unwrap() = Some(Arc::new(AiGatewayPermissionReviewer::new(
        capture.clone(),
        Arc::new(Clock(std::time::Instant::now())),
    )));
    let prefs = NativeModelPreferences::new(
        "source/main",
        NativeReasoningEffort::parse("high").unwrap(),
        true,
    )
    .unwrap();
    let runtime = NativeConversationRuntime::new(owner, prefs, None).unwrap();
    runtime.enqueue("real admitted root".into()).unwrap();
    let turn = block_on(runtime.start_next(1)).unwrap().unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let requests = capture.0.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let body = requests[0].to_string();
    assert_eq!(
        state.seen.lock().unwrap()[0].source_model(),
        Some("source/main")
    );
    for text in ["real admitted root", "original", "prepared"] {
        assert!(body.contains(text), "{body}");
    }
    for text in [
        "unproven user role",
        "untrusted assistant prose",
        "skipped",
        "archive",
        "source/main",
    ] {
        assert!(!body.contains(text), "{body}");
    }
}
