use super::*;
use crate::{
    NativeConfiguredPermissionRules, NativeConversation, NativeConversationTurn,
    NativeModelCapabilities, NativeModelSnapshot, NativePermissionActionPreparer,
    NativePermissionController, NativePreparedPermissionAction, NativeReasoningEffort,
    NativeUndoBudget, NativeWorkspaceContexts, PermissionMode, PermissionPromptDecision,
    PermissionPromptError, PermissionPrompter,
};
use futures_core::Stream;
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, Engine, ModelEvent, PermissionError, PermissionInvocation,
    PermissionRequest, SessionId, SessionIncarnationId, StopReason,
};
use machine_god_testkit::{InMemorySessionStore, ModelProviderStep, ScriptedModelProvider};
use std::{path::PathBuf, sync::atomic::AtomicU64, task::Waker};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct NoEffects;
impl NativePermissionActionPreparer for NoEffects {
    fn prepare<'a>(
        &'a self,
        _: &'a PermissionRequest,
        _: PermissionInvocation<'a>,
        _: CancellationToken,
    ) -> BoxFuture<'a, std::result::Result<Box<dyn NativePreparedPermissionAction>, PermissionError>>
    {
        panic!("provider-only test cannot prepare tools")
    }
}
impl PermissionPrompter for NoEffects {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, std::result::Result<PermissionPromptDecision, PermissionPromptError>> {
        panic!("provider-only test cannot prompt")
    }
}

struct Fixture {
    path: PathBuf,
    workspace: NativeWorkspaceAuthority,
    contexts: Arc<NativeWorkspaceContexts>,
    scheduler: ManagedScheduler,
    registry: NativePrincipalRegistry,
    permissions: Arc<NativePermissionController>,
    provider: ScriptedModelProvider,
    engine: Engine,
}
impl Fixture {
    fn new(steps: Vec<ModelProviderStep>) -> Self {
        let path = std::env::temp_dir().join(format!(
            "mg-managed-conversation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let primary = path.join("primary");
        let state = path.join("state");
        std::fs::create_dir(&primary).unwrap();
        std::fs::create_dir(&state).unwrap();
        let workspace = NativeWorkspaceAuthority::open_blocking(
            std::fs::File::open(&primary).unwrap().into(),
            primary,
            Some(std::fs::File::open(&state).unwrap().into()),
            state,
            vec![],
            false,
        )
        .unwrap();
        let provider = ScriptedModelProvider::new("test", steps);
        let permissions = Arc::new(NativePermissionController::new(
            Arc::new(NoEffects),
            Arc::new(NoEffects),
        ));
        let engine = Engine::builder()
            .provider(provider.clone())
            .shared_permission_handler(permissions.clone())
            .session_store(InMemorySessionStore::default())
            .build()
            .unwrap();
        Self {
            path,
            workspace,
            permissions,
            provider,
            engine,
            contexts: Arc::new(NativeWorkspaceContexts::new()),
            scheduler: ManagedScheduler::new(
                super::super::scheduler::SchedulerLimits::new(1, 2, 2).unwrap(),
            ),
            registry: NativePrincipalRegistry::new(2, Arc::new(NativeUndoBudget::default()))
                .unwrap(),
        }
    }

    fn conversation(&self, id: &str) -> (NativeConversation, ManagedConversationOwner) {
        let session = self
            .engine
            .create_session(
                SessionId::new(id).unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            )
            .unwrap();
        NativeConversation::from_session(session)
            .unwrap()
            .with_permission_controller(
                &self.permissions,
                NativePermissionPolicySnapshot::new(
                    PermissionMode::Ask,
                    Arc::new(NativeConfiguredPermissionRules::default()),
                ),
            )
            .unwrap()
            .with_managed_execution(
                &self.registry,
                self.scheduler.clone(),
                1,
                &self.workspace,
                &self.contexts,
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}

fn start(conversation: &NativeConversation) -> Result<NativeConversationTurn> {
    let preferences =
        NativeModelPreferences::new("selected-model", NativeReasoningEffort::default(), false)
            .unwrap();
    block_on(conversation.prompt_with_model(
        "managed work".into(),
        NativeModelSnapshot::new(&preferences, &NativeModelCapabilities::default()),
        100,
    ))
}

fn poll_until_pending(turn: &mut NativeConversationTurn) {
    for _ in 0..64 {
        match Pin::new(&mut *turn).poll_next(&mut Context::from_waker(Waker::noop())) {
            Poll::Pending => return,
            Poll::Ready(Some(Ok(_))) => {}
            other => panic!("expected live pending turn: {other:?}"),
        }
    }
    panic!("bounded scripted poll exhausted");
}

fn finish(turn: NativeConversationTurn) {
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(!events.is_empty());
    assert!(events.iter().all(std::result::Result::is_ok), "{events:?}");
}

fn completed() -> ModelProviderStep {
    ModelProviderStep::events([ModelEvent::Stop {
        reason: StopReason::Completed,
    }])
}

#[test]
fn actual_conversation_provider_pending_holds_quota_and_finalization_releases_execution_only() {
    let fixture = Fixture::new(vec![ModelProviderStep::pending(), completed()]);
    let (a, a_owner) = fixture.conversation("a");
    let (b, b_owner) = fixture.conversation("b");
    let mut first = start(&a).unwrap();
    let mut second = start(&b).unwrap();
    assert!(fixture.provider.requests().is_empty());
    assert_eq!(fixture.scheduler.snapshot().executing, 0);
    poll_until_pending(&mut first);
    poll_until_pending(&mut second);
    assert_eq!(fixture.provider.requests().len(), 1);
    assert_eq!(fixture.scheduler.snapshot().executing, 1);
    assert_eq!(fixture.scheduler.snapshot().queued, 1);
    first.handle().cancel();
    finish(first);
    finish(second);
    assert_eq!(fixture.provider.requests().len(), 2);
    assert_eq!(fixture.scheduler.snapshot().settling, 2);
    for owner in [&a_owner, &b_owner] {
        owner.take_settlement().unwrap().1.complete().unwrap();
    }
    assert_eq!(fixture.scheduler.snapshot().settling, 0);
    drop((a_owner, b_owner));
    assert_eq!(fixture.scheduler.snapshot().residents, 0);
}

#[test]
fn unfinished_settlement_rejects_next_checkpoint_even_after_custody_transfer() {
    let fixture = Fixture::new(vec![completed(), completed()]);
    let (conversation, owner) = fixture.conversation("a");
    finish(start(&conversation).unwrap());
    let before = conversation.record();
    assert!(matches!(
        start(&conversation),
        Err(NativeConversationError::ManagedAdmission)
    ));
    let (_, settlement) = owner.take_settlement().unwrap();
    assert!(matches!(
        start(&conversation),
        Err(NativeConversationError::ManagedAdmission)
    ));
    assert_eq!(conversation.record(), before);
    settlement.complete().unwrap();
    finish(start(&conversation).unwrap());
    owner.take_settlement().unwrap().1.complete().unwrap();
    assert_eq!(fixture.provider.requests().len(), 2);
}

#[test]
fn outer_owner_drop_cancels_real_turn_after_settlement_transfers() {
    let fixture = Fixture::new(vec![ModelProviderStep::pending()]);
    let (conversation, owner) = fixture.conversation("a");
    let mut turn = start(&conversation).unwrap();
    poll_until_pending(&mut turn);
    let (_, settlement) = owner.take_settlement().unwrap();
    drop(owner);
    assert!(turn.handle().is_cancelled());
    finish(turn);
    settlement.complete().unwrap();
    assert_eq!(fixture.scheduler.snapshot().residents, 0);
    let before = conversation.record();
    assert!(matches!(
        start(&conversation),
        Err(NativeConversationError::ManagedAdmission)
    ));
    assert_eq!(conversation.record(), before);
}

#[test]
fn missing_model_snapshot_and_expired_owner_fail_before_prompt_publication() {
    let fixture = Fixture::new(vec![]);
    let (conversation, owner) = fixture.conversation("a");
    let before = conversation.record();
    assert!(matches!(
        block_on(conversation.prompt("no captured model".into(), 100)),
        Err(NativeConversationError::ManagedAdmission)
    ));
    drop(owner);
    assert!(matches!(
        start(&conversation),
        Err(NativeConversationError::ManagedAdmission)
    ));
    assert_eq!(conversation.record(), before);
    assert!(fixture.provider.requests().is_empty());
}
