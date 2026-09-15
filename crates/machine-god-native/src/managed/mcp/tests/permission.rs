use super::*;
use crate::{
    NativeAutoPermissionAssessment, NativeAutoPermissionReview, NativeAutoPermissionReviewError,
    NativePermissionActionPreparer, NativePermissionAutomaticOutcome,
    NativePermissionConfiguredOutcome, NativePermissionContexts, NativePermissionController,
    NativePermissionExecutionProof, NativePermissionReviewer, NativePermissionRuleKey,
    NativePermissionTargetAuthority, NativePermissionTargetTool, NativePreparedPermissionAction,
    PermissionPromptDecision, PermissionPromptError, PermissionPrompter,
};
use machine_god_core::{
    Capability, PermissionError, PermissionExecutionAdmission, PermissionInvocation,
    PermissionRequest, PermissionRequestId, PermissionRisk, PreparedToolCall, ToolCall, ToolName,
    ToolOutput, ToolSpec,
};
use serde_json::Value;
use std::sync::atomic::AtomicUsize;

struct Reviewer;
impl NativePermissionReviewer for Reviewer {
    fn review<'a>(
        &'a self,
        _: NativeAutoPermissionReview<'a>,
        _: CancellationToken,
    ) -> BoxFuture<
        'a,
        std::result::Result<NativeAutoPermissionAssessment, NativeAutoPermissionReviewError>,
    > {
        Box::pin(async { panic!("builtin fixture does not use MCP reviewer") })
    }
}
struct Prompt;
impl PermissionPrompter for Prompt {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, std::result::Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async { panic!("yolo fixture must not prompt") })
    }
}
#[derive(Default)]
struct Hooks {
    prepared: AtomicUsize,
    admitted: AtomicUsize,
    executed: AtomicUsize,
    closed: Mutex<Vec<(SessionId, SessionIncarnationId, TurnId)>>,
    retire_on_bind: AtomicBool,
    owner: Mutex<Weak<NativePrincipalMcpOwner>>,
}
struct Builtin(Arc<Hooks>);
impl Tool for Builtin {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("helper_builtin").unwrap(),
            description: "fixture".into(),
            input_schema: json!({}),
        }
    }
    fn prepare(&self, call: ToolCall) -> std::result::Result<PreparedToolCall, ToolError> {
        Ok(PreparedToolCall::without_authority(call.arguments)
            .require_tool_permission(call.name, call.id))
    }
    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            self.0.executed.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput::success(json!({"ok":true})))
        })
    }
}
struct BuiltinPreparer(Arc<Hooks>);
impl NativePermissionActionPreparer for BuiltinPreparer {
    fn close_turn(&self, s: &SessionId, i: &SessionIncarnationId, t: &TurnId) {
        self.0
            .closed
            .lock()
            .unwrap()
            .push((s.clone(), i.clone(), t.clone()));
    }
    fn prepare<'a>(
        &'a self,
        _: &'a PermissionRequest,
        _: PermissionInvocation<'a>,
        _: CancellationToken,
    ) -> BoxFuture<'a, std::result::Result<Box<dyn NativePreparedPermissionAction>, PermissionError>>
    {
        Box::pin(async move {
            self.0.prepared.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(BuiltinAction(self.0.clone())) as Box<dyn NativePreparedPermissionAction>)
        })
    }
}
struct BuiltinAction(Arc<Hooks>);
impl NativePreparedPermissionAction for BuiltinAction {
    fn saved_rule_key(&self) -> Option<&NativePermissionRuleKey> {
        None
    }
    fn grant_key(&self) -> Option<&NativePermissionRuleKey> {
        None
    }
    fn is_file_mutation(&self) -> bool {
        true
    }
    fn configured_outcome(
        &self,
        _: &NativeConfiguredPermissionRules,
    ) -> std::result::Result<NativePermissionConfiguredOutcome, PermissionError> {
        Ok(NativePermissionConfiguredOutcome::Unresolved)
    }
    fn allows_without_review(&self, _: PermissionMode) -> bool {
        true
    }
    fn automatic_review(
        &self,
        _: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<NativePermissionAutomaticOutcome, PermissionError>> {
        Box::pin(async { Ok(NativePermissionAutomaticOutcome::Allow) })
    }
    fn bind_execution(
        self: Box<Self>,
        proof: NativePermissionExecutionProof,
    ) -> std::result::Result<Box<dyn PermissionExecutionAdmission>, PermissionError> {
        if self.0.retire_on_bind.load(Ordering::Acquire) {
            let owner = self.0.owner.lock().unwrap().upgrade().unwrap();
            owner.retire();
        }
        Ok(Box::new(BuiltinAdmission {
            hooks: self.0,
            proof,
        }))
    }
}
struct BuiltinAdmission {
    hooks: Arc<Hooks>,
    proof: NativePermissionExecutionProof,
}
impl PermissionExecutionAdmission for BuiltinAdmission {
    fn admit(self: Box<Self>) -> std::result::Result<(), PermissionError> {
        self.proof.revalidate()?;
        self.hooks.admitted.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
fn inputs(
    f: &Fixture,
    contexts: Arc<NativeMcpContexts>,
    hooks: Arc<Hooks>,
) -> NativePrincipalMcpPermissionInputs {
    NativePrincipalMcpPermissionInputs {
        builtins: Arc::new(
            NativePermissionTargetAuthority::new(
                std::fs::File::open(f.path.join("primary")).unwrap(),
                f.path.join("primary").to_str().unwrap().into(),
                vec![NativePermissionTargetTool::Ordinary(Arc::new(Builtin(
                    hooks.clone(),
                )))],
            )
            .unwrap(),
        ),
        builtin_preparer: Arc::new(BuiltinPreparer(hooks)),
        contexts,
        review_contexts: Arc::new(NativePermissionContexts::new()),
        reviewer: Arc::new(Reviewer),
        workspace: f.path.join("primary").to_str().unwrap().into(),
        workspace_contexts: None,
    }
}
fn request(s: &Session, t: &Turn) -> PermissionRequest {
    PermissionRequest {
        id: PermissionRequestId::new("request").unwrap(),
        session_id: s.id(),
        session_incarnation_id: s.incarnation_id(),
        turn_id: t.id().clone(),
        capability: Capability::Custom {
            name: "fixture".into(),
            details: json!({}),
        },
        risk: PermissionRisk::Low,
        reason: "fixture".into(),
    }
}
fn invocation<'a>(
    name: &'a ToolName,
    id: &'a ToolCallId,
    args: &'a Value,
) -> PermissionInvocation<'a> {
    PermissionInvocation {
        tool_name: name,
        call_id: id,
        arguments: args,
    }
}

#[test]
fn permission_bundle_rejects_context_and_runtime_mismatch_without_closing_anything() {
    let f = Fixture::new(1);
    let (contexts, a) = runtime();
    let (foreign, b) = runtime();
    assert!(NativePrincipalMcpPermissions::new(&a, inputs(&f, foreign, Arc::default())).is_err());
    let permissions =
        NativePrincipalMcpPermissions::new(&a, inputs(&f, contexts, Arc::default())).unwrap();
    let engine = engine();
    let s = session(&engine, "a");
    let principal = f.principals.register(&s, 1, &f.workspace).unwrap();
    assert!(
        f.registry
            .register(&principal, &b, None, Some(&permissions))
            .is_err()
    );
    assert!(a.publication_checkpoint().is_ok());
    assert!(b.publication_checkpoint().is_ok());
    let owner = f
        .registry
        .register(&principal, &a, None, Some(&permissions))
        .unwrap();
    let weak = Arc::downgrade(&a);
    drop(a);
    assert!(
        weak.upgrade().is_some(),
        "outer permission owner retains actual runtime"
    );
    drop(permissions);
    assert!(
        weak.upgrade().is_none(),
        "principal registration retains no preparer/runtime cycle"
    );
    assert!(!owner.live());
}

#[test]
fn permission_preparation_is_inert_weak_and_original_action_rejects_retirement() {
    let f = Fixture::new(1);
    let engine = engine();
    let s = session(&engine, "a");
    let principal = f.principals.register(&s, 1, &f.workspace).unwrap();
    let (contexts, runtime) = runtime();
    let hooks = Arc::new(Hooks::default());
    let bundle =
        NativePrincipalMcpPermissions::new(&runtime, inputs(&f, contexts, hooks.clone())).unwrap();
    let owner = f
        .registry
        .register(&principal, &runtime, None, Some(&bundle))
        .unwrap();
    let turn = block_on(s.prompt("a")).unwrap();
    let principal_turn = begin(&principal, &turn);
    let route = owner.begin_turn(&principal_turn).unwrap();
    let router = f.registry.permission_preparer();
    let request = request(&s, &turn);
    let name = ToolName::new("helper_builtin").unwrap();
    let id = ToolCallId::new("call").unwrap();
    let args = json!({});
    let before = Arc::strong_count(&runtime);
    let pending = router.prepare(
        &request,
        invocation(&name, &id, &args),
        CancellationToken::new(),
    );
    assert_eq!(hooks.prepared.load(Ordering::SeqCst), 0);
    assert_eq!(Arc::strong_count(&runtime), before);
    drop(pending);
    let action = block_on(router.prepare(
        &request,
        invocation(&name, &id, &args),
        CancellationToken::new(),
    ))
    .unwrap();
    assert!(action.is_file_mutation());
    assert!(action.allows_without_review(PermissionMode::Yolo));
    let pending = router.prepare(
        &request,
        invocation(&name, &id, &args),
        CancellationToken::new(),
    );
    drop(route);
    assert!(block_on(pending).is_err());
    assert!(!action.allows_without_review(PermissionMode::Yolo));
    assert!(
        action
            .configured_outcome(&NativeConfiguredPermissionRules::default())
            .is_err()
    );
    assert!(block_on(action.automatic_review(CancellationToken::new())).is_err());
    assert_eq!(hooks.prepared.load(Ordering::SeqCst), 1);
    assert_eq!(hooks.closed.lock().unwrap().len(), 1);
}

#[test]
fn cancelled_turn_cleanup_routes_original_preparer_without_touching_siblings() {
    let f = Fixture::new(2);
    let engine = engine();
    let a = session(&engine, "a");
    let b = session(&engine, "b");
    let pa = f.principals.register(&a, 1, &f.workspace).unwrap();
    let pb = f.principals.register(&b, 1, &f.workspace).unwrap();
    let (ca, ra) = runtime();
    let (cb, rb) = runtime();
    let ha = Arc::new(Hooks::default());
    let hb = Arc::new(Hooks::default());
    let ba = NativePrincipalMcpPermissions::new(&ra, inputs(&f, ca, ha.clone())).unwrap();
    let bb = NativePrincipalMcpPermissions::new(&rb, inputs(&f, cb, hb.clone())).unwrap();
    let oa = f.registry.register(&pa, &ra, None, Some(&ba)).unwrap();
    let ob = f.registry.register(&pb, &rb, None, Some(&bb)).unwrap();
    let ta = block_on(a.prompt("a")).unwrap();
    let tb = block_on(b.prompt("b")).unwrap();
    let ga = begin(&pa, &ta);
    let gb = begin(&pb, &tb);
    let ma = oa.begin_turn(&ga).unwrap();
    let _mb = ob.begin_turn(&gb).unwrap();
    assert!(ta.handle().cancel());
    assert!(!ga.stamp().is_live());
    f.registry
        .permission_preparer()
        .close_turn(&a.id(), &a.incarnation_id(), ta.id());
    assert!(ha.closed.lock().unwrap().is_empty());
    assert!(hb.closed.lock().unwrap().is_empty());
    assert!(ob.live());
    drop(ga);
    drop(ma);
    assert_eq!(ha.closed.lock().unwrap().len(), 1);
    oa.retire();
    assert!(hb.closed.lock().unwrap().is_empty());
    assert!(rb.publication_checkpoint().is_ok());
}

#[test]
fn builtin_permission_preflight_preserves_helper_and_claims_only_at_actual_execution() {
    use futures_util::StreamExt;
    use machine_god_core::{ModelEvent, StopReason};
    use machine_god_testkit::ModelProviderStep;
    for retire_on_bind in [false, true] {
        let f = Fixture::new(1);
        let hooks = Arc::new(Hooks::default());
        hooks
            .retire_on_bind
            .store(retire_on_bind, Ordering::Release);
        let controller = Arc::new(NativePermissionController::new(
            Arc::new(f.registry.permission_preparer()),
            Arc::new(Prompt),
        ));
        let provider = ScriptedModelProvider::new(
            "test",
            [
                ModelProviderStep::events([
                    ModelEvent::ToolCall {
                        call: ToolCall {
                            id: ToolCallId::new("call").unwrap(),
                            name: ToolName::new("helper_builtin").unwrap(),
                            arguments: json!({}),
                        },
                    },
                    ModelEvent::Stop {
                        reason: StopReason::ToolCalls,
                    },
                ]),
                ModelProviderStep::events([ModelEvent::Stop {
                    reason: StopReason::Completed,
                }]),
            ],
        );
        let engine = Engine::builder()
            .provider(provider)
            .session_store(InMemorySessionStore::default())
            .shared_permission_handler(controller.clone())
            .tool(f.registry.wrap_tool(Arc::new(Builtin(hooks.clone()))))
            .build()
            .unwrap();
        let s = session(&engine, "a");
        let principal = f.principals.register(&s, 1, &f.workspace).unwrap();
        let (contexts, runtime) = runtime();
        let permissions =
            NativePrincipalMcpPermissions::new(&runtime, inputs(&f, contexts, hooks.clone()))
                .unwrap();
        let owner = f
            .registry
            .register(&principal, &runtime, None, Some(&permissions))
            .unwrap();
        *hooks.owner.lock().unwrap() = Arc::downgrade(&owner);
        let policy = NativePermissionPolicySnapshot::new(
            PermissionMode::Yolo,
            Arc::new(NativeConfiguredPermissionRules::default()),
        );
        let permission_session = controller.register(s.clone(), policy.clone()).unwrap();
        let mut turn = block_on(s.prompt("a")).unwrap();
        let _permission = permission_session.begin_turn(&turn, policy).unwrap();
        let guard = begin(&principal, &turn);
        let _route = owner.begin_turn(&guard).unwrap();
        block_on(async {
            while let Some(event) = turn.next().await {
                event.unwrap();
            }
        });
        assert_eq!(hooks.prepared.load(Ordering::SeqCst), 1);
        assert_eq!(
            hooks.admitted.load(Ordering::SeqCst),
            usize::from(!retire_on_bind)
        );
        assert_eq!(
            hooks.executed.load(Ordering::SeqCst),
            usize::from(!retire_on_bind)
        );
    }
}

#[test]
fn ambiguous_public_ids_neither_prepare_nor_close_a_sibling_actual_allocation() {
    let f = Fixture::new(2);
    let ea = engine();
    let eb = engine();
    let a = session(&ea, "same");
    let b = session(&eb, "same");
    let pa = f.principals.register(&a, 1, &f.workspace).unwrap();
    let pb = f.principals.register(&b, 1, &f.workspace).unwrap();
    let (ca, ra) = runtime();
    let (cb, rb) = runtime();
    let ha = Arc::new(Hooks::default());
    let hb = Arc::new(Hooks::default());
    let ba = NativePrincipalMcpPermissions::new(&ra, inputs(&f, ca, ha.clone())).unwrap();
    let bb = NativePrincipalMcpPermissions::new(&rb, inputs(&f, cb, hb.clone())).unwrap();
    let oa = f.registry.register(&pa, &ra, None, Some(&ba)).unwrap();
    let ob = f.registry.register(&pb, &rb, None, Some(&bb)).unwrap();
    let ta = block_on(a.prompt("a")).unwrap();
    let tb = block_on(b.prompt("b")).unwrap();
    assert_eq!(ta.id(), tb.id());
    let ga = begin(&pa, &ta);
    let gb = begin(&pb, &tb);
    let ma = oa.begin_turn(&ga).unwrap();
    let mb = ob.begin_turn(&gb).unwrap();
    let router = f.registry.permission_preparer();
    let request = request(&a, &ta);
    let name = ToolName::new("helper_builtin").unwrap();
    let id = ToolCallId::new("same-call").unwrap();
    let args = json!({});
    assert!(
        block_on(router.prepare(
            &request,
            invocation(&name, &id, &args),
            CancellationToken::new()
        ))
        .is_err()
    );
    router.close_turn(&a.id(), &a.incarnation_id(), ta.id());
    assert_eq!(ha.prepared.load(Ordering::SeqCst), 0);
    assert_eq!(hb.prepared.load(Ordering::SeqCst), 0);
    assert!(ha.closed.lock().unwrap().is_empty());
    assert!(hb.closed.lock().unwrap().is_empty());
    drop(ma);
    assert_eq!(ha.closed.lock().unwrap().len(), 1);
    assert!(hb.closed.lock().unwrap().is_empty());
    drop(ga);
    pa.retire();
    oa.retire();
    // Only B now remains, but the old A callback still carries identical IDs.
    let b_request = self::request(&b, &tb);
    let action = block_on(router.prepare(
        &b_request,
        invocation(&name, &id, &args),
        CancellationToken::new(),
    ))
    .unwrap();
    router.close_turn(&a.id(), &a.incarnation_id(), ta.id());
    assert!(action.allows_without_review(PermissionMode::Yolo));
    assert!(hb.closed.lock().unwrap().is_empty());
    router.close_turn(&b.id(), &b.incarnation_id(), tb.id());
    assert_eq!(ha.closed.lock().unwrap().len(), 1);
    assert!(hb.closed.lock().unwrap().is_empty());
    drop(mb);
    assert_eq!(hb.closed.lock().unwrap().len(), 1);
}
