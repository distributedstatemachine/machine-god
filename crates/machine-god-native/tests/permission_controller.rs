use std::future::poll_fn;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_executor::block_on;
use futures_util::task::noop_waker;
use machine_god_core::*;
use machine_god_native::*;
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider};
use serde_json::json;

#[path = "permission_controller/file_composition.rs"]
mod file_composition;

#[derive(Clone)]
struct Behavior {
    configured: NativePermissionConfiguredOutcome,
    automatic: Result<NativePermissionAutomaticOutcome, PermissionError>,
    file: bool,
    bypass: bool,
}

struct Adapter {
    behavior: Mutex<Behavior>,
    prepared: AtomicUsize,
    reviewed: Arc<AtomicUsize>,
    closed: AtomicUsize,
}

struct Action {
    behavior: Behavior,
    key: NativePermissionRuleKey,
    reviewed: Arc<AtomicUsize>,
}

impl NativePermissionActionPreparer for Adapter {
    fn close_turn(&self, _: &SessionId, _: &SessionIncarnationId, _: &TurnId) {
        self.closed.fetch_add(1, Ordering::SeqCst);
    }
    fn prepare<'a>(
        &'a self,
        _request: &'a PermissionRequest,
        _invocation: PermissionInvocation<'a>,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn NativePreparedPermissionAction>, PermissionError>> {
        Box::pin(async move {
            self.prepared.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Action {
                behavior: self.behavior.lock().unwrap().clone(),
                key: key("exact"),
                reviewed: Arc::clone(&self.reviewed),
            }) as Box<dyn NativePreparedPermissionAction>)
        })
    }
}

impl NativePreparedPermissionAction for Action {
    fn saved_rule_key(&self) -> Option<&NativePermissionRuleKey> {
        Some(&self.key)
    }
    fn grant_key(&self) -> Option<&NativePermissionRuleKey> {
        Some(&self.key)
    }
    fn is_file_mutation(&self) -> bool {
        self.behavior.file
    }
    fn configured_outcome(
        &self,
        _: &NativeConfiguredPermissionRules,
    ) -> Result<NativePermissionConfiguredOutcome, PermissionError> {
        Ok(self.behavior.configured)
    }
    fn allows_without_review(&self, _: PermissionMode) -> bool {
        self.behavior.bypass
    }
    fn automatic_review(
        &self,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<NativePermissionAutomaticOutcome, PermissionError>> {
        Box::pin(async move {
            self.reviewed.fetch_add(1, Ordering::SeqCst);
            self.behavior.automatic.clone()
        })
    }
    fn bind_execution(
        self: Box<Self>,
        proof: NativePermissionExecutionProof,
    ) -> Result<Box<dyn PermissionExecutionAdmission>, PermissionError> {
        Ok(Box::new(proof))
    }
}

struct Prompter {
    calls: AtomicUsize,
    pending: AtomicBool,
    decision: Mutex<PermissionPromptDecision>,
}

impl PermissionPrompter for Prompter {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            poll_fn(|_| {
                if self.pending.load(Ordering::SeqCst) {
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
            Ok(*self.decision.lock().unwrap())
        })
    }
}

struct Fixture {
    _engine: Engine,
    session: Session,
    owner: Arc<NativePermissionSession>,
    controller: Arc<NativePermissionController>,
    adapter: Arc<Adapter>,
    prompt: Arc<Prompter>,
}

fn key(value: &str) -> NativePermissionRuleKey {
    NativePermissionRuleKey::new(NativePermissionRuleKind::StructuredTool, value).unwrap()
}

impl Fixture {
    fn new(mode: PermissionMode) -> Self {
        Self::with_store(mode, InMemorySessionStore::default())
    }

    fn with_store(mode: PermissionMode, store: impl SessionStore) -> Self {
        let adapter = Arc::new(Adapter {
            behavior: Mutex::new(Behavior {
                configured: NativePermissionConfiguredOutcome::Unresolved,
                automatic: Ok(NativePermissionAutomaticOutcome::Allow),
                file: false,
                bypass: false,
            }),
            prepared: AtomicUsize::new(0),
            reviewed: Arc::new(AtomicUsize::new(0)),
            closed: AtomicUsize::new(0),
        });
        let prompt = Arc::new(Prompter {
            calls: AtomicUsize::new(0),
            pending: AtomicBool::new(false),
            decision: Mutex::new(PermissionPromptDecision::AllowOnce),
        });
        let controller = Arc::new(NativePermissionController::new(
            adapter.clone(),
            prompt.clone(),
        ));
        let engine = Engine::builder()
            .session_store(store)
            .provider(ScriptedModelProvider::new("test", []))
            .shared_permission_handler(controller.clone())
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("session").unwrap(),
                SessionIncarnationId::new("life").unwrap(),
            )
            .unwrap();
        let owner = controller
            .register(
                session.clone(),
                NativePermissionPolicySnapshot::new(mode, Arc::default()),
            )
            .unwrap();
        Self {
            _engine: engine,
            session,
            owner,
            controller,
            adapter,
            prompt,
        }
    }

    fn turn(&self) -> (Turn, NativePermissionTurn) {
        let turn = block_on(self.session.prompt("user request")).unwrap();
        let registration = self.owner.begin_turn(&turn, self.owner.snapshot()).unwrap();
        (turn, registration)
    }

    fn request(&self, turn: &Turn) -> PermissionRequest {
        PermissionRequest {
            id: PermissionRequestId::new("permission").unwrap(),
            session_id: self.session.id(),
            session_incarnation_id: self.session.incarnation_id(),
            turn_id: turn.id().clone(),
            capability: Capability::Filesystem {
                access: FilesystemAccess::Read,
                path: "file".to_owned(),
            },
            risk: PermissionRisk::Low,
            reason: "private reason".to_owned(),
        }
    }

    fn authorize(&self, turn: &Turn) -> Result<PermissionAuthorization, PermissionError> {
        let name = ToolName::new("read_file").unwrap();
        let call = ToolCallId::new("call").unwrap();
        block_on(self.controller.authorize_invocation(
            self.request(turn),
            PermissionInvocation {
                tool_name: &name,
                call_id: &call,
                arguments: &json!({"path":"file"}),
            },
        ))
    }

    fn save(&self, value: &str, decision: NativePermissionRuleDecision) {
        let proposal = self
            .owner
            .propose_rule_change(NativePermissionRuleChange::Set {
                key: key(value),
                display_identity: "display only".to_owned(),
                decision,
            })
            .unwrap();
        block_on(self.owner.confirm_rule_change(proposal)).unwrap();
    }
}

fn allowed(authorization: PermissionAuthorization) {
    assert_eq!(
        authorization.decision,
        PermissionDecision::Allow {
            scope: PermissionGrantScope::Once
        }
    );
    authorization.admission.unwrap().admit().unwrap();
}

fn denied(authorization: PermissionAuthorization) {
    assert!(matches!(
        authorization.decision,
        PermissionDecision::Deny { .. }
    ));
    assert!(authorization.admission.is_none());
    drop(authorization);
}

#[test]
fn authorization_is_inert_and_requires_actual_invocation_and_live_route() {
    let fixture = Fixture::new(PermissionMode::Ask);
    let (turn, registration) = fixture.turn();
    assert!(block_on(fixture.controller.authorize(fixture.request(&turn))).is_err());
    let name = ToolName::new("read_file").unwrap();
    let call = ToolCallId::new("call").unwrap();
    let arguments = json!({"path":"file"});
    drop(fixture.controller.authorize_invocation(
        fixture.request(&turn),
        PermissionInvocation {
            tool_name: &name,
            call_id: &call,
            arguments: &arguments,
        },
    ));
    assert_eq!(fixture.adapter.prepared.load(Ordering::SeqCst), 0);
    allowed(fixture.authorize(&turn).unwrap());
    drop(registration);
    assert!(fixture.authorize(&turn).is_err());
}

#[test]
fn pending_prompt_cannot_resurrect_grants_after_reset() {
    let fixture = Fixture::new(PermissionMode::Ask);
    let (turn, _registration) = fixture.turn();
    fixture.prompt.pending.store(true, Ordering::SeqCst);
    *fixture.prompt.decision.lock().unwrap() = PermissionPromptDecision::AllowSession;
    let name = ToolName::new("read_file").unwrap();
    let call = ToolCallId::new("call").unwrap();
    let arguments = json!({"path":"file"});
    let mut authorization = fixture.controller.authorize_invocation(
        fixture.request(&turn),
        PermissionInvocation {
            tool_name: &name,
            call_id: &call,
            arguments: &arguments,
        },
    );
    assert!(
        authorization
            .as_mut()
            .poll(&mut Context::from_waker(&noop_waker()))
            .is_pending()
    );
    fixture.owner.reset().unwrap();
    fixture.prompt.pending.store(false, Ordering::SeqCst);
    assert!(block_on(authorization).is_err());
    allowed(fixture.authorize(&turn).unwrap());
    assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn reset_revokes_delayed_admission_without_changing_taken_yolo_mode() {
    let fixture = Fixture::new(PermissionMode::Ask);
    let (turn, _registration) = fixture.turn();
    let authorization = fixture.authorize(&turn).unwrap();
    fixture.owner.reset().unwrap();
    assert!(authorization.admission.unwrap().admit().is_err());
    let fixture = Fixture::new(PermissionMode::Yolo);
    let (turn, _registration) = fixture.turn();
    let authorization = fixture.authorize(&turn).unwrap();
    fixture.owner.reset().unwrap();
    allowed(authorization);
    allowed(fixture.authorize(&turn).unwrap());
    assert_eq!(fixture.owner.snapshot().mode(), PermissionMode::Ask);
    assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn turn_grants_expire_but_session_grants_survive_turn_closure() {
    for scope in [
        PermissionPromptDecision::AllowTurn,
        PermissionPromptDecision::AllowSession,
    ] {
        let fixture = Fixture::new(PermissionMode::Ask);
        *fixture.prompt.decision.lock().unwrap() = scope;
        let (turn, registration) = fixture.turn();
        allowed(fixture.authorize(&turn).unwrap());
        allowed(fixture.authorize(&turn).unwrap());
        assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), 1);
        drop(registration);
        drop(turn);
        let (turn, _registration) = fixture.turn();
        allowed(fixture.authorize(&turn).unwrap());
        assert_eq!(
            fixture.prompt.calls.load(Ordering::SeqCst),
            if scope == PermissionPromptDecision::AllowTurn {
                2
            } else {
                1
            }
        );
    }
}

#[test]
fn saved_allow_satisfies_configured_ask_but_never_configured_deny() {
    let fixture = Fixture::new(PermissionMode::Ask);
    let (turn, _registration) = fixture.turn();
    fixture.save("exact", NativePermissionRuleDecision::Allow);
    fixture.adapter.behavior.lock().unwrap().configured = NativePermissionConfiguredOutcome::Ask;
    allowed(fixture.authorize(&turn).unwrap());
    fixture.adapter.behavior.lock().unwrap().configured = NativePermissionConfiguredOutcome::Deny;
    denied(fixture.authorize(&turn).unwrap());
    assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn explicit_configured_ask_uses_human_until_an_exact_capability_grant_exists() {
    let fixture = Fixture::new(PermissionMode::Auto);
    fixture.adapter.behavior.lock().unwrap().configured = NativePermissionConfiguredOutcome::Ask;
    *fixture.prompt.decision.lock().unwrap() = PermissionPromptDecision::AllowSession;
    let (turn, _registration) = fixture.turn();
    allowed(fixture.authorize(&turn).unwrap());
    allowed(fixture.authorize(&turn).unwrap());
    assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.adapter.reviewed.load(Ordering::SeqCst), 0);
}

#[test]
fn auto_ask_and_transport_failure_deny_for_replanning_without_human_fallback() {
    let fixture = Fixture::new(PermissionMode::Auto);
    let (turn, _registration) = fixture.turn();
    for result in [
        Ok(NativePermissionAutomaticOutcome::Ask),
        Err(PermissionError::new("private", "private")),
    ] {
        fixture.adapter.behavior.lock().unwrap().automatic = result;
        denied(fixture.authorize(&turn).unwrap());
    }
    assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), 0);
    fixture.adapter.behavior.lock().unwrap().automatic =
        Ok(NativePermissionAutomaticOutcome::Allow);
    allowed(fixture.authorize(&turn).unwrap());
}

#[test]
fn yolo_bypasses_ordinary_rules_but_file_saved_deny_remains_authoritative() {
    let fixture = Fixture::new(PermissionMode::Yolo);
    let (turn, _registration) = fixture.turn();
    fixture.save("exact", NativePermissionRuleDecision::Deny);
    fixture.adapter.behavior.lock().unwrap().configured = NativePermissionConfiguredOutcome::Deny;
    allowed(fixture.authorize(&turn).unwrap());
    fixture.adapter.behavior.lock().unwrap().file = true;
    denied(fixture.authorize(&turn).unwrap());
}

#[test]
fn saved_rule_revocation_rejects_previously_authorized_execution_and_reset_preserves_rules() {
    let fixture = Fixture::new(PermissionMode::Ask);
    let (turn, _registration) = fixture.turn();
    fixture.save("exact", NativePermissionRuleDecision::Allow);
    let authorization = fixture.authorize(&turn).unwrap();
    let rules =
        NativeSessionPermissionRules::from_metadata(&fixture.session.record().metadata).unwrap();
    let proposal = fixture
        .owner
        .propose_rule_change(NativePermissionRuleChange::Revoke {
            id: rules.rules()[0].id(),
        })
        .unwrap();
    block_on(fixture.owner.confirm_rule_change(proposal)).unwrap();
    assert!(authorization.admission.unwrap().admit().is_err());
    fixture.save("exact", NativePermissionRuleDecision::Deny);
    fixture.owner.reset().unwrap();
    denied(fixture.authorize(&turn).unwrap());
}

#[test]
fn confirmation_is_owner_bound_single_use_and_pins_only_the_affected_rule_generation() {
    let fixture = Fixture::new(PermissionMode::Ask);
    let (_turn, _registration) = fixture.turn();
    let change = || NativePermissionRuleChange::Set {
        key: key("exact"),
        display_identity: "display".to_owned(),
        decision: NativePermissionRuleDecision::Allow,
    };
    let first = fixture.owner.propose_rule_change(change()).unwrap();
    let stale = fixture.owner.propose_rule_change(change()).unwrap();
    fixture.save("unrelated", NativePermissionRuleDecision::Deny);
    block_on(fixture.owner.confirm_rule_change(first)).unwrap();
    assert!(block_on(fixture.owner.confirm_rule_change(stale)).is_err());
    let other = Fixture::new(PermissionMode::Ask);
    let proposal = fixture.owner.propose_rule_change(change()).unwrap();
    assert!(block_on(other.owner.confirm_rule_change(proposal)).is_err());
    let rules =
        NativeSessionPermissionRules::from_metadata(&fixture.session.record().metadata).unwrap();
    assert_eq!(rules.rules().len(), 2);
}

#[test]
fn active_rule_editor_preserves_transcript_and_unrelated_metadata_without_cloning_snapshot() {
    let fixture = Fixture::new(PermissionMode::Ask);
    let (turn, _registration) = fixture.turn();
    let before = fixture.session.record_snapshot();
    assert!(Arc::ptr_eq(&before, &fixture.session.record_snapshot()));
    let editor = turn.metadata_editor("other").unwrap();
    let snapshot = block_on(editor.read_entry()).unwrap();
    block_on(editor.compare_exchange(snapshot, Some(json!("preserve")))).unwrap();
    fixture.save("exact", NativePermissionRuleDecision::Allow);
    let after = fixture.session.record_snapshot();
    assert_eq!(before.messages, after.messages);
    assert!(!Arc::ptr_eq(&before, &after));
    assert_eq!(after.metadata.get("other"), Some(&json!("preserve")));
    assert!(
        !before
            .metadata
            .contains_key(NATIVE_SESSION_PERMISSION_RULES_KEY)
    );
}

#[test]
fn foreign_turn_binding_and_cancelled_turn_execution_are_rejected() {
    let fixture = Fixture::new(PermissionMode::Ask);
    let (turn, _registration) = fixture.turn();
    let authorization = fixture.authorize(&turn).unwrap();
    assert!(turn.handle().cancel());
    assert!(authorization.admission.unwrap().admit().is_err());
    let other = Fixture::new(PermissionMode::Ask);
    assert!(
        other
            .owner
            .begin_turn(&turn, other.owner.snapshot())
            .is_err()
    );
}

#[test]
fn dropped_restrictive_save_blocks_authority_and_old_proof_cannot_revive_after_reconciliation() {
    use machine_god_testkit::{SessionStoreScript, SessionStoreStep};
    let store = InMemorySessionStore::configured(
        std::collections::BTreeMap::default(),
        SessionStoreScript {
            loads: None,
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Pending,
            ]),
        },
        64,
    );
    let fixture = Fixture::with_store(PermissionMode::Ask, store);
    let (turn, _registration) = fixture.turn();
    fixture.save("exact", NativePermissionRuleDecision::Allow);
    let authorization = fixture.authorize(&turn).unwrap();
    let proposal = fixture
        .owner
        .propose_rule_change(NativePermissionRuleChange::Set {
            key: key("exact"),
            display_identity: "deny".to_owned(),
            decision: NativePermissionRuleDecision::Deny,
        })
        .unwrap();
    let mut save = fixture.owner.confirm_rule_change(proposal);
    assert!(
        save.as_mut()
            .poll(&mut Context::from_waker(&noop_waker()))
            .is_pending()
    );
    assert!(fixture.authorize(&turn).is_err());
    drop(save);
    assert!(fixture.authorize(&turn).is_err());
    block_on(fixture.owner.reconcile_rules()).unwrap();
    assert!(authorization.admission.unwrap().admit().is_err());
    allowed(fixture.authorize(&turn).unwrap());
}

#[test]
fn external_canonical_rule_edit_is_seen_by_final_execution_check() {
    let fixture = Fixture::new(PermissionMode::Ask);
    let (turn, _registration) = fixture.turn();
    let authorization = fixture.authorize(&turn).unwrap();
    let rules = NativeSessionPermissionRules::default()
        .apply_set(
            &key("exact"),
            "deny",
            NativePermissionRuleDecision::Deny,
            None,
        )
        .unwrap();
    let editor = turn
        .metadata_editor(NATIVE_SESSION_PERMISSION_RULES_KEY)
        .unwrap();
    let snapshot = block_on(editor.read_entry()).unwrap();
    block_on(editor.compare_exchange(snapshot, Some(rules.to_value()))).unwrap();
    assert!(authorization.admission.unwrap().admit().is_err());
}

struct Executed(Arc<AtomicUsize>);
impl Tool for Executed {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("echo").unwrap(),
            description: "test tool".to_owned(),
            input_schema: json!({}),
        }
    }
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        Ok(PreparedToolCall::new(
            Capability::Tool {
                name: call.name,
                call_id: call.id,
                arguments: call.arguments.clone(),
            },
            call.arguments,
        ))
    }
    fn execute(
        &self,
        _: ToolContext,
        _: serde_json::Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput::success(json!({"executed":true})))
        })
    }
}

#[test]
fn composed_runtime_enforces_taken_mode_and_uses_reset_mode_for_next_queued_job() {
    use futures_util::StreamExt;
    use machine_god_testkit::ModelProviderStep;
    let fixture = Fixture::new(PermissionMode::Ask);
    *fixture.prompt.decision.lock().unwrap() = PermissionPromptDecision::Deny;
    let executed = Arc::new(AtomicUsize::new(0));
    let tool_round = || {
        ModelProviderStep::events([
            ModelEvent::ToolCall {
                call: ToolCall {
                    id: ToolCallId::new("reused-call").unwrap(),
                    name: ToolName::new("echo").unwrap(),
                    arguments: json!({}),
                },
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])
    };
    let finish = || {
        ModelProviderStep::events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])
    };
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::default())
        .provider(ScriptedModelProvider::new(
            "test",
            [tool_round(), finish(), tool_round(), finish()],
        ))
        .shared_permission_handler(fixture.controller.clone())
        .tool(Executed(executed.clone()))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("runtime").unwrap(),
            SessionIncarnationId::new("runtime-life").unwrap(),
        )
        .unwrap();
    let conversation = NativeConversation::from_session(session)
        .unwrap()
        .with_permission_controller(
            &fixture.controller,
            NativePermissionPolicySnapshot::new(PermissionMode::Yolo, Arc::default()),
        )
        .unwrap();
    let runtime = NativeConversationRuntime::new(
        conversation,
        NativeModelPreferences::new("test/model", NativeReasoningEffort::default(), false).unwrap(),
        None,
    )
    .unwrap();
    runtime.enqueue("first".into()).unwrap();
    runtime.enqueue("second".into()).unwrap();
    let mut first = block_on(runtime.start_next(100)).unwrap().unwrap();
    runtime.permissions().unwrap().reset().unwrap();
    block_on(async {
        while let Some(event) = first.next().await {
            event.unwrap();
        }
    });
    assert_eq!(executed.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), 0);
    let mut second = block_on(runtime.start_next(101)).unwrap().unwrap();
    block_on(async {
        while let Some(event) = second.next().await {
            event.unwrap();
        }
    });
    assert_eq!(executed.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.prompt.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.adapter.closed.load(Ordering::SeqCst), 2);
}
