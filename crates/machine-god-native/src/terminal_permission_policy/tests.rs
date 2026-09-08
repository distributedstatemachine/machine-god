use super::*;
use crate::{
    NativePermissionActionPreparer, NativePermissionPolicySnapshot, NativePermissionSession,
    NativePermissionTurn, NativePreparedPermissionAction, PermissionMode, PermissionPromptDecision,
    PermissionPromptError, PermissionPrompter,
};
use futures_executor::block_on;
use machine_god_core::{
    BoxFuture, Engine, PermissionError, PermissionInvocation, PermissionRequest, Session,
    SessionId, SessionIncarnationId, ToolCallId, Turn, TurnId,
};
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider};
use std::time::Duration;

struct NoEffects;
impl NativePermissionActionPreparer for NoEffects {
    fn prepare<'a>(
        &'a self,
        _: &'a PermissionRequest,
        _: PermissionInvocation<'a>,
        _: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn NativePreparedPermissionAction>, PermissionError>> {
        Box::pin(async { panic!("policy observation must not prepare effects") })
    }
}
impl PermissionPrompter for NoEffects {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async { panic!("policy observation must not prompt") })
    }
}

pub(crate) struct Fixture {
    pub(crate) controller: Arc<NativePermissionController>,
    pub(crate) owner: Arc<NativePermissionSession>,
    pub(crate) session: Session,
    _engine: Engine,
}
impl Fixture {
    pub(crate) fn new(mode: PermissionMode, sandbox: NativeSandboxMode) -> Self {
        let controller = Arc::new(NativePermissionController::new(
            Arc::new(NoEffects),
            Arc::new(NoEffects),
        ));
        let engine = Engine::builder()
            .session_store(InMemorySessionStore::default())
            .permission_handler(machine_god_testkit::ScriptedPermissionHandler::new([]))
            .provider(ScriptedModelProvider::new("test", []))
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("policy-session").unwrap(),
                SessionIncarnationId::new("policy-life").unwrap(),
            )
            .unwrap();
        let owner = controller
            .register(
                session.clone(),
                NativePermissionPolicySnapshot::new(mode, Arc::default())
                    .with_sandbox_mode(sandbox),
            )
            .unwrap();
        Self {
            controller,
            owner,
            session,
            _engine: engine,
        }
    }
    pub(crate) fn turn(&self) -> (Turn, NativePermissionTurn, ToolContext) {
        let turn = block_on(self.session.prompt("test")).unwrap();
        let registration = self.owner.begin_turn(&turn, self.owner.snapshot()).unwrap();
        let context = ToolContext {
            session_id: self.session.id(),
            session_incarnation_id: self.session.incarnation_id(),
            turn_id: turn.id().clone(),
            call_id: ToolCallId::new("call").unwrap(),
        };
        (turn, registration, context)
    }
}

fn capture(
    policy: &NativeTerminalPermissionPolicy,
    context: &ToolContext,
) -> Result<Arc<NativeSandboxLaunch>, NativeSandboxError> {
    policy.capture_on_worker(
        context,
        Instant::now() + Duration::from_secs(5),
        &CancellationToken::new(),
    )
}

#[test]
fn binding_is_once_weak_and_fail_closed() {
    let fixture = Fixture::new(PermissionMode::Ask, NativeSandboxMode::None);
    let (_turn, _registration, context) = fixture.turn();
    let policy = NativeTerminalPermissionPolicy::new(vec![], None).unwrap();
    assert_eq!(
        capture(&policy, &context).unwrap_err(),
        NativeSandboxError::Unavailable
    );
    let count = Arc::strong_count(&fixture.controller);
    policy.bind_controller(&fixture.controller).unwrap();
    assert_eq!(Arc::strong_count(&fixture.controller), count);
    assert!(policy.bind_controller(&fixture.controller).is_err());
    assert_eq!(
        capture(&policy, &context).unwrap().effective(),
        NativeSandboxMode::None
    );
    drop(fixture);
    assert_eq!(
        capture(&policy, &context).unwrap_err(),
        NativeSandboxError::Unavailable
    );
}

#[test]
fn exact_live_context_deadline_and_cancellation_are_required() {
    let fixture = Fixture::new(PermissionMode::Ask, NativeSandboxMode::None);
    let (turn, registration, context) = fixture.turn();
    let policy = NativeTerminalPermissionPolicy::new(vec![], None).unwrap();
    policy.bind_controller(&fixture.controller).unwrap();
    let mut wrong = context.clone();
    wrong.session_id = SessionId::new("wrong").unwrap();
    assert!(capture(&policy, &wrong).is_err());
    wrong = context.clone();
    wrong.session_incarnation_id = SessionIncarnationId::new("wrong").unwrap();
    assert!(capture(&policy, &wrong).is_err());
    wrong = context.clone();
    wrong.turn_id = TurnId::new("wrong").unwrap();
    assert!(capture(&policy, &wrong).is_err());
    assert_eq!(
        policy
            .capture_on_worker(&context, Instant::now(), &CancellationToken::new())
            .unwrap_err(),
        NativeSandboxError::Timeout
    );
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        policy
            .capture_on_worker(&context, Instant::now() + Duration::from_secs(1), &token)
            .unwrap_err(),
        NativeSandboxError::Cancelled
    );
    drop(registration);
    assert!(capture(&policy, &context).is_err());
    drop(turn);
    let (_new, _registration, next) = fixture.turn();
    assert!(capture(&policy, &context).is_err());
    assert!(capture(&policy, &next).is_ok());
}

#[test]
fn taken_yolo_needs_no_os_authority_and_retains_configured_os_after_reset() {
    let fixture = Fixture::new(PermissionMode::Yolo, NativeSandboxMode::Os);
    let (_turn, _registration, context) = fixture.turn();
    let policy = NativeTerminalPermissionPolicy::new(vec![], None).unwrap();
    policy.bind_controller(&fixture.controller).unwrap();
    fixture.owner.reset().unwrap();
    fixture.owner.set_sandbox_mode(NativeSandboxMode::None);
    let snapshot = capture(&policy, &context).unwrap();
    assert_eq!(snapshot.configured(), NativeSandboxMode::Os);
    assert_eq!(snapshot.effective(), NativeSandboxMode::None);
}

#[test]
fn taken_os_never_uses_later_yolo_or_missing_executable_fallback() {
    let fixture = Fixture::new(PermissionMode::Auto, NativeSandboxMode::Os);
    let (_turn, _registration, context) = fixture.turn();
    let policy = NativeTerminalPermissionPolicy::new(vec![], None).unwrap();
    policy.bind_controller(&fixture.controller).unwrap();
    fixture.owner.set_mode(PermissionMode::Yolo);
    fixture.owner.set_sandbox_mode(NativeSandboxMode::None);
    fixture.owner.reset().unwrap();
    assert!(matches!(
        capture(&policy, &context),
        Err(NativeSandboxError::Unavailable | NativeSandboxError::Unsupported)
    ));
}

#[test]
fn constructors_are_inert_bounded_and_redacted() {
    let descriptor = File::open("/dev/null").unwrap();
    // No stat/canonicalization occurs: this deliberately nonexistent path and
    // non-directory descriptor can be retained, but never launch OS isolation.
    let root =
        NativeSandboxRoot::new(descriptor, "/definitely-not-a-real-policy-root".into()).unwrap();
    assert!(
        NativeTerminalPermissionPolicy::new(vec![root.clone(); MAX_NATIVE_SANDBOX_ROOTS], None)
            .is_ok()
    );
    assert!(matches!(
        NativeTerminalPermissionPolicy::new(vec![root.clone(); MAX_NATIVE_SANDBOX_ROOTS + 1], None),
        Err(NativeSandboxError::Invalid)
    ));
    let policy = NativeTerminalPermissionPolicy::new(vec![root], None).unwrap();
    assert!(!format!("{policy:?}").contains("definitely"));
}
