use super::*;
use crate::{NativePermissionRuleChange, NativePermissionRuleKind, PermissionPromptError};
use futures_executor::block_on;
use machine_god_core::{
    Engine, SessionId, SessionIncarnationId, SessionRecord, SessionRevision, SessionStore,
    SessionStoreError,
};
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler};
use std::future::poll_fn;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

struct NoEffects;

#[test]
fn saved_prompt_proposals_retire_with_reset_cancel_drop_and_inbox_scope() {
    for retirement in 0..4 {
        let f = Fixture::new();
        let turn = block_on(f.owner.session.prompt("proposal source")).unwrap();
        let registration = f
            .owner
            .begin_turn(&turn, f.owner.snapshot().unwrap())
            .unwrap();
        let (attempt, epoch, rules_epoch) = f.owner.attempt(turn.id()).unwrap();
        let source = NativePermissionRulePrompt::new(
            &f.owner,
            &attempt,
            epoch,
            rules_epoch,
            NativePermissionRuleKey::new(
                crate::NativePermissionRuleKind::StructuredTool,
                "native exact identity",
            )
            .unwrap(),
        );
        let saves_before_proposal = f.store.saves.load(Ordering::SeqCst);
        let (bridge, mut inbox) = crate::NativeInteractivePromptBridge::new(
            crate::NativeInteractivePromptLimits::default(),
        )
        .unwrap();
        inbox
            .activate(machine_god_core::BackgroundOutputOwner::new(
                f.owner.session.id(),
                f.owner.session.incarnation_id(),
            ))
            .unwrap();
        let request = PermissionRequest {
            id: machine_god_core::PermissionRequestId::new("source").unwrap(),
            session_id: f.owner.session.id(),
            session_incarnation_id: f.owner.session.incarnation_id(),
            turn_id: turn.id().clone(),
            capability: machine_god_core::Capability::Custom {
                name: "not the canonical identity".into(),
                details: serde_json::Value::Null,
            },
            risk: machine_god_core::PermissionRisk::Low,
            reason: "display only".into(),
        };
        let mut future = bridge.prompt_with_rule(request, Some(source));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut cx).is_pending());
        let Poll::Ready(Some(view)) = inbox.poll_prompt(&mut cx) else {
            panic!("displayed native prompt");
        };
        let proposal = inbox
            .propose_rule_change(view.token(), NativePermissionRuleDecision::Allow)
            .unwrap();
        match retirement {
            0 => {
                f.owner.reset().unwrap();
            }
            1 => {
                inbox.cancel(view.token()).unwrap();
            }
            2 => {
                inbox.deactivate();
            }
            _ => {
                drop(future);
                assert!(block_on(f.owner.confirm_rule_change(proposal)).is_err());
                drop(registration);
                continue;
            }
        }
        assert!(block_on(f.owner.confirm_rule_change(proposal)).is_err());
        assert_eq!(f.store.saves.load(Ordering::SeqCst), saves_before_proposal);
        drop(future);
        drop(registration);
    }
}

#[test]
fn reset_rejects_idle_proposal_but_does_not_claim_rollback_after_publication_started() {
    let f = Fixture::new();
    let old = f.proposal();
    f.owner.reset().unwrap();
    assert!(block_on(f.owner.confirm_rule_change(old)).is_err());
    assert_eq!(f.store.saves.load(Ordering::SeqCst), 0);
    let proposal = f.proposal();
    f.store.pending.store(true, Ordering::SeqCst);
    let mut save = f.owner.confirm_rule_change(proposal);
    assert!(
        save.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(f.store.saves.load(Ordering::SeqCst), 1);
    f.owner.reset().unwrap();
    drop(save);
    assert!(lock(&f.owner.state).uncertain_rules);
    assert!(
        f.owner
            .propose_rule_change(NativePermissionRuleChange::Revoke { id: 1 })
            .is_err()
    );
}
impl NativePermissionActionPreparer for NoEffects {
    fn prepare<'a>(
        &'a self,
        _: &'a PermissionRequest,
        _: PermissionInvocation<'a>,
        _: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn NativePreparedPermissionAction>, PermissionError>> {
        panic!("lifecycle control must not prepare an effect")
    }
}
impl PermissionPrompter for NoEffects {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        panic!("lifecycle control must not prompt")
    }
}
#[derive(Default)]
struct Store {
    inner: InMemorySessionStore,
    pending: AtomicBool,
    saves: AtomicUsize,
}
impl SessionStore for Store {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        self.inner.load(id)
    }
    fn save(
        &self,
        record: SessionRecord,
        revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        Box::pin(async move {
            self.saves.fetch_add(1, Ordering::SeqCst);
            poll_fn(|_| {
                if self.pending.load(Ordering::SeqCst) {
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
            self.inner.save(record, revision).await
        })
    }
}
struct Fixture {
    _engine: Engine,
    controller: NativePermissionController,
    owner: Arc<NativePermissionSession>,
    gate: Arc<LifecycleGate>,
    store: Arc<Store>,
}
impl Fixture {
    fn new() -> Self {
        let controller = NativePermissionController::new(Arc::new(NoEffects), Arc::new(NoEffects));
        let store = Arc::new(Store::default());
        let engine = Engine::builder()
            .provider(ScriptedModelProvider::new("fixture", []))
            .permission_handler(ScriptedPermissionHandler::new([]))
            .shared_session_store(store.clone())
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("lifecycle").unwrap(),
                SessionIncarnationId::new("same-life").unwrap(),
            )
            .unwrap();
        let owner = controller
            .register(
                session,
                NativePermissionPolicySnapshot::new(PermissionMode::Auto, Arc::default()),
            )
            .unwrap();
        let gate = LifecycleGate::new();
        owner.bind_lifecycle(&gate).unwrap();
        Self {
            _engine: engine,
            controller,
            owner,
            gate,
            store,
        }
    }
    fn proposal(&self) -> NativePermissionRuleProposal {
        self.owner
            .propose_rule_change(NativePermissionRuleChange::Set {
                key: NativePermissionRuleKey::new(
                    NativePermissionRuleKind::StructuredTool,
                    "exact",
                )
                .unwrap(),
                display_identity: "display".into(),
                decision: NativePermissionRuleDecision::Allow,
            })
            .unwrap()
    }
}

#[test]
fn quiescent_snapshot_observes_settled_admitted_selection_without_public_admission() {
    let f = Fixture::new();
    let admitted = f.owner.acquire_lifecycle().unwrap();
    let guard = f.gate.begin_quiescence().unwrap();
    assert!(f.owner.snapshot_quiescent(&guard).is_err());
    // Simulate a synchronous selection setter paused after its successful
    // admission and before its state lock, then resuming behind the fence.
    {
        let mut state = lock(&f.owner.state);
        state.policy.mode = PermissionMode::Yolo;
        state.policy.sandbox_mode = NativeSandboxMode::Os;
    }
    assert!(f.owner.snapshot_quiescent(&guard).is_err());
    drop(admitted);
    let snapshot = f.owner.snapshot_quiescent(&guard).unwrap();
    assert_eq!(snapshot.mode(), PermissionMode::Yolo);
    assert_eq!(snapshot.sandbox_mode(), NativeSandboxMode::Os);
    assert_eq!(snapshot.effective_sandbox_mode(), NativeSandboxMode::None);
    assert!(Arc::ptr_eq(
        &snapshot.configured,
        &lock(&f.owner.state).policy.configured,
    ));
    assert!(f.owner.snapshot().is_err());
    assert!(f.owner.set_mode(PermissionMode::Ask).is_err());
    let other = LifecycleGate::new();
    let foreign = other.begin_quiescence().unwrap();
    assert!(f.owner.snapshot_quiescent(&foreign).is_err());
    assert_eq!(lock(&f.controller.routes).len(), 1);
    f.owner.retire();
    assert!(f.owner.snapshot_quiescent(&guard).is_err());
    assert!(lock(&f.controller.routes).is_empty());
    assert_eq!(snapshot.mode(), PermissionMode::Yolo);
    drop(guard);
    assert!(f.owner.snapshot().is_err());
}

#[test]
fn gate_attachment_cannot_overtake_an_already_admitted_unbound_control() {
    let f = Fixture::new();
    let controller = NativePermissionController::new(Arc::new(NoEffects), Arc::new(NoEffects));
    let owner = controller
        .register(
            f.owner.session.clone(),
            NativePermissionPolicySnapshot::new(PermissionMode::Auto, Arc::default()),
        )
        .unwrap();
    let admitted = owner.acquire_lifecycle().unwrap();
    let gate = LifecycleGate::new();
    assert!(owner.bind_lifecycle(&gate).is_err());
    assert!(owner.lifecycle.get().is_none());
    drop(admitted);
    owner.bind_lifecycle(&gate).unwrap();
    let quiescence = gate.begin_quiescence().unwrap();
    assert!(owner.snapshot().is_err());
    drop(quiescence);
    assert_eq!(owner.snapshot().unwrap().mode(), PermissionMode::Auto);
}

#[test]
fn public_turn_registration_retains_its_admission_until_exact_close() {
    let f = Fixture::new();
    let turn = block_on(f.owner.session.prompt("direct admitted turn")).unwrap();
    let registration = f
        .owner
        .begin_turn(&turn, f.owner.snapshot().unwrap())
        .unwrap();
    let mut quiescence = f.gate.begin_quiescence().unwrap();
    let mut wait = quiescence.wait_idle();
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(registration);
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    drop(wait);
    drop(quiescence);
    drop(turn);
    assert_eq!(f.owner.snapshot().unwrap().mode(), PermissionMode::Auto);
}

#[test]
fn retired_aliases_and_unpolled_rule_futures_cannot_reacquire_or_write() {
    let f = Fixture::new();
    let pending = f.owner.confirm_rule_change(f.proposal());
    assert_eq!(f.store.saves.load(Ordering::SeqCst), 0);
    f.gate.begin_quiescence().unwrap().try_retire().unwrap();
    f.owner.retire();
    assert!(f.owner.snapshot().is_err());
    assert!(f.owner.set_mode(PermissionMode::Yolo).is_err());
    assert!(f.owner.set_sandbox_mode(NativeSandboxMode::Os).is_err());
    assert!(f.owner.reset().is_err());
    assert!(block_on(pending).is_err());
    assert!(block_on(f.owner.reconcile_rules()).is_err());
    assert_eq!(f.store.saves.load(Ordering::SeqCst), 0);
    let replacement = f
        .controller
        .register(
            f.owner.session.clone(),
            NativePermissionPolicySnapshot::new(PermissionMode::Ask, Arc::default()),
        )
        .unwrap();
    f.owner.retire();
    drop(f.owner);
    assert_eq!(lock(&f.controller.routes).len(), 1);
    assert_eq!(replacement.snapshot().unwrap().mode(), PermissionMode::Ask);
}

#[test]
fn admitted_turn_binds_and_reconciles_during_quiescence_but_old_proof_never_revives() {
    let f = Fixture::new();
    let permit = f.gate.acquire().unwrap();
    let policy = f.owner.snapshot_admitted(&permit).unwrap();
    let turn = block_on(f.owner.session.prompt("admitted")).unwrap();
    let foreign = LifecycleGate::new().acquire().unwrap();
    assert!(f.owner.snapshot_admitted(&foreign).is_err());
    assert!(
        f.owner
            .begin_turn_admitted(&turn, policy.clone(), &foreign)
            .is_err()
    );
    let mut quiescence = f.gate.begin_quiescence().unwrap();
    assert!(f.owner.snapshot().is_err());
    assert!(f.owner.begin_turn(&turn, policy.clone()).is_err());
    let registration = f.owner.begin_turn_admitted(&turn, policy, &permit).unwrap();
    block_on(f.owner.reconcile_rules_admitted(&permit)).unwrap();
    let proof = NativePermissionExecutionProof {
        owner: Arc::downgrade(&f.owner),
        attempt: Arc::downgrade(&registration.attempt),
        epoch: Some(1),
        rules_epoch: Some(1),
        saved: None,
        saved_generation: None,
    };
    proof.revalidate().unwrap();
    let mut wait = quiescence.wait_idle();
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(wait);
    drop(registration);
    drop(turn);
    drop(permit);
    block_on(quiescence.wait_idle()).unwrap();
    quiescence.try_retire().unwrap();
    f.owner.retire();
    assert!(proof.revalidate().is_err());
    let replacement = f
        .controller
        .register(
            f.owner.session.clone(),
            NativePermissionPolicySnapshot::new(PermissionMode::Yolo, Arc::default()),
        )
        .unwrap();
    assert!(proof.revalidate().is_err());
    assert_eq!(replacement.snapshot().unwrap().mode(), PermissionMode::Yolo);
}

#[test]
fn pending_confirmed_save_spans_quiescence_and_abort_preserves_policy() {
    let f = Fixture::new();
    f.store.pending.store(true, Ordering::SeqCst);
    let mut save = f.owner.confirm_rule_change(f.proposal());
    assert!(
        save.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let mut quiescence = f.gate.begin_quiescence().unwrap();
    let mut wait = quiescence.wait_idle();
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(wait);
    assert!(f.owner.reset().is_err());
    drop(quiescence);
    assert_eq!(f.owner.snapshot().unwrap().mode(), PermissionMode::Auto);
    f.store.pending.store(false, Ordering::SeqCst);
    block_on(save).unwrap();
    assert_eq!(read_rules(&f.owner.session).unwrap().rules().len(), 1);
    let mut quiescence = f.gate.begin_quiescence().unwrap();
    block_on(quiescence.wait_idle()).unwrap();
    drop(quiescence);
    assert_eq!(f.owner.snapshot().unwrap().mode(), PermissionMode::Auto);
}

struct ReentrantWake {
    owner: Weak<NativePermissionSession>,
    wakes: AtomicUsize,
}
impl Wake for ReentrantWake {
    fn wake(self: Arc<Self>) {
        assert!(self.owner.upgrade().unwrap().snapshot().is_err());
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn dropped_rule_save_releases_permit_and_wakes_outside_policy_lock() {
    let f = Fixture::new();
    f.store.pending.store(true, Ordering::SeqCst);
    let mut save = f.owner.confirm_rule_change(f.proposal());
    assert!(
        save.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let mut quiescence = f.gate.begin_quiescence().unwrap();
    let wake = Arc::new(ReentrantWake {
        owner: Arc::downgrade(&f.owner),
        wakes: AtomicUsize::new(0),
    });
    let waker = Waker::from(wake.clone());
    let mut wait = quiescence.wait_idle();
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    drop(save);
    assert_eq!(wake.wakes.load(Ordering::SeqCst), 1);
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    drop(wait);
    drop(quiescence);
    assert_eq!(f.owner.snapshot().unwrap().mode(), PermissionMode::Auto);
    assert!(lock(&f.owner.state).uncertain_rules);
}

#[test]
fn configured_reload_changes_future_snapshots_without_rewriting_taken_policy_or_grants() {
    let f = Fixture::new();
    f.owner.set_sandbox_mode(NativeSandboxMode::Os).unwrap();
    let key = NativePermissionRuleKey::new(NativePermissionRuleKind::StructuredTool, "exact-grant")
        .unwrap();
    block_on(f.owner.confirm_rule_change(f.proposal())).unwrap();
    lock(&f.owner.state).grants.push(Grant {
        key: key.clone(),
        turn: None,
    });
    let old = f.owner.snapshot().unwrap();
    let turn = block_on(f.owner.session.prompt("taken job")).unwrap();
    let registration = f.owner.begin_turn(&turn, old.clone()).unwrap();
    let before_record = f.owner.session.record_snapshot();
    let rules = Arc::new(
        NativeConfiguredPermissionRules::new(vec![
            crate::NativeConfiguredPermissionRule::new(
                "read",
                "*",
                crate::NativeConfiguredPermissionDecision::Allow,
            )
            .unwrap(),
        ])
        .unwrap(),
    );
    f.owner.set_configured_rules(rules.clone()).unwrap();
    let new = f.owner.snapshot().unwrap();
    assert!(old.configured.rules().is_empty());
    assert!(registration.attempt.policy.configured.rules().is_empty());
    assert!(Arc::ptr_eq(&new.configured, &rules));
    assert_eq!(new.mode(), PermissionMode::Auto);
    assert_eq!(new.sandbox_mode(), NativeSandboxMode::Os);
    assert!(
        lock(&f.owner.state)
            .grants
            .iter()
            .any(|grant| grant.key == key)
    );
    assert_eq!(f.owner.session.record_snapshot(), before_record);
}

#[cfg(feature = "ai-gateway-http")]
#[test]
fn configured_reload_accepts_only_its_already_admitted_permit_during_quiescence() {
    let f = Fixture::new();
    let permit = f.gate.acquire().unwrap();
    let mut quiescence = f.gate.begin_quiescence().unwrap();
    let rules = Arc::new(
        NativeConfiguredPermissionRules::new(vec![
            crate::NativeConfiguredPermissionRule::new(
                "read",
                "*",
                crate::NativeConfiguredPermissionDecision::Deny,
            )
            .unwrap(),
        ])
        .unwrap(),
    );
    assert!(f.owner.set_configured_rules(rules.clone()).is_err());
    let foreign = LifecycleGate::new().acquire().unwrap();
    assert!(
        f.owner
            .set_configured_rules_admitted(&foreign, rules.clone())
            .is_err()
    );
    f.owner
        .set_configured_rules_admitted(&permit, rules.clone())
        .unwrap();
    assert!(f.owner.snapshot_quiescent(&quiescence).is_err());
    drop(permit);
    block_on(quiescence.wait_idle()).unwrap();
    assert!(Arc::ptr_eq(
        &f.owner.snapshot_quiescent(&quiescence).unwrap().configured,
        &rules
    ));
    quiescence.try_retire().unwrap();
    f.owner.retire();
    assert!(f.owner.set_configured_rules(rules).is_err());
}
