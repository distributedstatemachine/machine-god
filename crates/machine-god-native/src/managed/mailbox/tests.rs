use super::super::principal::{NativePrincipalRegistry, NativePrincipalTurn};
use super::*;
use crate::file_undo::NativeUndoBudget;
use crate::{NativeModelPreferences, NativePermissionPolicySnapshot, NativeWorkspaceAuthority};
use futures_core::Stream;
use futures_executor::block_on;
use machine_god_core::{
    Engine, ManagedFailureCode, ManagedResultStatus, ModelEvent, Session, SessionId,
    SessionIncarnationId, StopReason, SubagentTool, ToolCall, ToolCallId, ToolName, Turn,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
};
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
    task::{Wake, Waker},
};

struct Capture(Arc<Mutex<VecDeque<ManagedSubagentInvocation>>>);
impl ManagedSubagentAuthority for Capture {
    fn execute(
        &self,
        invocation: ManagedSubagentInvocation,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ManagedSubagentResult, Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push_back(invocation);
            std::future::pending().await
        })
    }
}
struct Fixture {
    registry: NativePrincipalRegistry,
    workspace: NativeWorkspaceAuthority,
    path: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "mg-managed-mailbox-{}-{}",
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
        Self {
            registry: NativePrincipalRegistry::new(64, Arc::new(NativeUndoBudget::default()))
                .unwrap(),
            workspace,
            path,
        }
    }
    fn mailbox(&self, count: usize) -> ManagedMailbox {
        ManagedMailbox::new(
            self.registry.requester(),
            MailboxLimits {
                requests: count,
                bytes: count * OPERATION_BYTES,
            },
        )
        .unwrap()
    }
    fn invocation(&self, id: &str) -> (Admission, ManagedSubagentInvocation) {
        let captured = Arc::new(Mutex::new(VecDeque::new()));
        let engine = Engine::builder()
            .provider(ScriptedModelProvider::new("test", [ModelProviderStep::events([
                ModelEvent::ToolCall { call: ToolCall { id: ToolCallId::new("same-provider-id").unwrap(),
                    name: ToolName::new("subagent").unwrap(), arguments: serde_json::json!({"command":{"create":{"name":id,"mode":"persistent"}}}) } },
                ModelEvent::Stop { reason: StopReason::ToolCalls },
            ])]))
            .permission_handler(ScriptedPermissionHandler::new([
                machine_god_testkit::PermissionStep::Decision(machine_god_core::PermissionDecision::Allow {
                    scope: machine_god_core::PermissionGrantScope::Once,
                }),
            ]))
            .session_store(InMemorySessionStore::default())
            .tool(SubagentTool::new(Capture(captured.clone())))
            .build().unwrap();
        let session = engine
            .create_session(
                SessionId::new(id).unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            )
            .unwrap();
        let principal = self
            .registry
            .register(&session, 1, &self.workspace)
            .unwrap();
        let mut turn = block_on(session.prompt("create")).unwrap();
        let guard = principal
            .begin_turn(
                &turn,
                NativePermissionPolicySnapshot::new(
                    crate::PermissionMode::Ask,
                    Arc::new(crate::NativeConfiguredPermissionRules::default()),
                ),
                NativeModelPreferences::new(
                    "model",
                    crate::NativeReasoningEffort::parse("high").unwrap(),
                    false,
                )
                .unwrap(),
                None,
            )
            .unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..100 {
            let _ = Pin::new(&mut turn).poll_next(&mut cx);
            if let Some(invocation) = captured.lock().unwrap().pop_front() {
                return (
                    Admission {
                        _guard: guard,
                        _turn: turn,
                        _session: session,
                        _engine: engine,
                    },
                    invocation,
                );
            }
        }
        panic!("actual admitted invocation not produced");
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}
struct Admission {
    _guard: NativePrincipalTurn,
    _turn: Turn,
    _session: Session,
    _engine: Engine,
}
fn poll<T>(future: Pin<&mut dyn Future<Output = T>>) -> Poll<T> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}
fn next(mailbox: &ManagedMailbox) -> ManagedMailboxJob {
    let Poll::Ready(Some(job)) = mailbox.poll_next(&mut Context::from_waker(Waker::noop())) else {
        panic!("queued job")
    };
    job
}
fn result() -> ManagedSubagentResult {
    ManagedSubagentResult {
        ok: false,
        operation_id: "operation".into(),
        child_id: None,
        status: ManagedResultStatus::Rejected,
        error_code: Some(ManagedFailureCode::InvalidState),
        retryable: false,
        requested: None,
        cursor: None,
    }
}

#[test]
fn construction_is_inert_and_first_poll_claims_actual_invocation_once() {
    let fixture = Fixture::new();
    let mailbox = fixture.mailbox(1);
    let requester = mailbox.requester();
    let (_admission, invocation) = fixture.invocation("principal");
    let mut future = requester.execute(invocation, CancellationToken::new());
    assert_eq!(mailbox.usage(), MailboxUsage::default());
    assert!(
        mailbox
            .poll_next(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert!(poll(future.as_mut()).is_pending());
    let job = next(&mailbox);
    assert!(job.lease().is_live());
    assert_eq!(job.context().session_id.as_str(), "principal");
    assert!(
        fixture
            .registry
            .claim(job.invocation.as_ref().unwrap())
            .is_err()
    );
    assert!(matches!(job.command(), ManagedSubagentCommand::Create(_)));
    job.complete(Ok(result()));
    assert!(matches!(poll(future.as_mut()), Poll::Ready(Ok(_))));
    assert_eq!(mailbox.usage(), MailboxUsage::default());
}

#[test]
fn foreign_retired_and_cancelled_admission_fails_without_queue_acceptance() {
    let fixture = Fixture::new();
    let mailbox = fixture.mailbox(2);
    let requester = mailbox.requester();
    let (admission, invocation) = fixture.invocation("retired");
    drop(admission);
    assert_eq!(
        block_on(requester.execute(invocation, CancellationToken::new())).unwrap_err(),
        Error::Unavailable
    );
    let (_admission, invocation) = fixture.invocation("cancelled");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        block_on(requester.execute(invocation, cancellation)).unwrap_err(),
        Error::Cancelled
    );
    let other = Fixture::new();
    let (_foreign, invocation) = other.invocation("foreign");
    assert_eq!(
        block_on(requester.execute(invocation, CancellationToken::new())).unwrap_err(),
        Error::Unavailable
    );
    assert_eq!(mailbox.usage(), MailboxUsage::default());
}

#[test]
fn fifo_jobs_and_slow_responses_share_count_and_byte_pressure() {
    let fixture = Fixture::new();
    let mailbox = fixture.mailbox(2);
    let requester = mailbox.requester();
    let (_a, first) = fixture.invocation("first");
    let (_b, second) = fixture.invocation("second");
    let (_c, third) = fixture.invocation("third");
    let mut a = requester.execute(first, CancellationToken::new());
    let mut b = requester.execute(second, CancellationToken::new());
    assert!(poll(a.as_mut()).is_pending());
    assert!(poll(b.as_mut()).is_pending());
    let first = next(&mailbox);
    assert_eq!(first.context().session_id.as_str(), "first");
    first.complete(Ok(result()));
    let second = next(&mailbox);
    assert_eq!(second.context().session_id.as_str(), "second");
    assert_eq!(mailbox.usage().requests, 2);
    assert_eq!(
        block_on(requester.execute(third, CancellationToken::new())).unwrap_err(),
        Error::ResourceLimit
    );
    assert!(matches!(poll(a.as_mut()), Poll::Ready(Ok(_))));
    assert_eq!(mailbox.usage().requests, 1);
    drop(b);
    assert!(second.observer_gone());
    assert_eq!(mailbox.usage().bytes, OPERATION_BYTES);
    second.complete(Ok(result()));
    assert_eq!(mailbox.usage(), MailboxUsage::default());
}

#[test]
fn byte_capacity_and_invalid_response_remain_charged_through_observation() {
    let fixture = Fixture::new();
    let mailbox = ManagedMailbox::new(
        fixture.registry.requester(),
        MailboxLimits {
            requests: 2,
            bytes: OPERATION_BYTES,
        },
    )
    .unwrap();
    let requester = mailbox.requester();
    let (_a, first) = fixture.invocation("first");
    let (_b, second) = fixture.invocation("second");
    let mut future = requester.execute(first, CancellationToken::new());
    assert!(poll(future.as_mut()).is_pending());
    assert_eq!(
        block_on(requester.execute(second, CancellationToken::new())).unwrap_err(),
        Error::ResourceLimit
    );
    let mut invalid = result();
    invalid.ok = true;
    next(&mailbox).complete(Ok(invalid));
    assert_eq!(mailbox.usage().bytes, OPERATION_BYTES);
    assert!(matches!(
        poll(future.as_mut()),
        Poll::Ready(Err(Error::InvalidArguments))
    ));
    assert_eq!(mailbox.usage(), MailboxUsage::default());
}

#[test]
fn submission_cancellation_or_observer_drop_does_not_discard_mutation_custody() {
    let fixture = Fixture::new();
    let mailbox = fixture.mailbox(1);
    let requester = mailbox.requester();
    let (_admission, invocation) = fixture.invocation("principal");
    let cancellation = CancellationToken::new();
    let mut future = requester.execute(invocation, cancellation.clone());
    assert!(poll(future.as_mut()).is_pending());
    cancellation.cancel();
    drop(future);
    let job = next(&mailbox);
    assert!(job.observer_gone());
    assert!(job.cancellation().is_cancelled());
    assert!(job.lease().is_live());
    assert_eq!(mailbox.usage().requests, 1);
    job.complete(Ok(result()));
    assert_eq!(mailbox.usage().requests, 0);
}

#[test]
fn manager_close_rejects_replies_but_dequeued_work_keeps_its_lease_and_charge() {
    let fixture = Fixture::new();
    let mailbox = fixture.mailbox(2);
    let requester = mailbox.requester();
    let (_a, a) = fixture.invocation("dequeued");
    let (_b, b) = fixture.invocation("queued");
    let mut a = requester.execute(a, CancellationToken::new());
    let mut b = requester.execute(b, CancellationToken::new());
    assert!(poll(a.as_mut()).is_pending());
    assert!(poll(b.as_mut()).is_pending());
    let job = next(&mailbox);
    mailbox.close();
    assert!(matches!(
        poll(a.as_mut()),
        Poll::Ready(Err(Error::Unavailable))
    ));
    assert!(matches!(
        poll(b.as_mut()),
        Poll::Ready(Err(Error::Unavailable))
    ));
    assert_eq!(mailbox.usage().requests, 1);
    assert!(job.lease().is_live());
    assert!(job.observer_gone());
    job.complete(Ok(result()));
    assert_eq!(mailbox.usage().requests, 0);
    assert!(matches!(
        mailbox.poll_next(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(None)
    ));
}

#[test]
fn dropping_manager_breaks_weak_routes_without_invalidating_dequeued_custody() {
    let fixture = Fixture::new();
    let mailbox = fixture.mailbox(1);
    let requester = mailbox.requester();
    let wake = mailbox.wake_handle();
    let (_a, invocation) = fixture.invocation("principal");
    let mut future = requester.execute(invocation, CancellationToken::new());
    assert!(poll(future.as_mut()).is_pending());
    let job = next(&mailbox);
    let budget = mailbox.shared.budget.clone();
    drop(mailbox);
    assert!(requester.shared.upgrade().is_none());
    assert!(wake.0.upgrade().is_none());
    wake.wake();
    assert!(matches!(
        poll(future.as_mut()),
        Poll::Ready(Err(Error::Unavailable))
    ));
    assert_eq!(budget.usage.lock().unwrap().requests, 1);
    drop(job);
    assert_eq!(budget.usage.lock().unwrap().requests, 0);
}

#[test]
fn invalid_results_are_rejected_and_spare_capacity_is_not_retained() {
    let mut invalid = result();
    invalid.operation_id = "x".repeat(machine_god_core::MAX_SUBAGENT_OUTPUT_BYTES + 1);
    assert!(response::bounded_result(Ok(invalid)).is_err());
    let mut valid = result();
    valid.operation_id.reserve(1024 * 1024);
    let normalized = response::bounded_result(Ok(valid)).unwrap();
    assert!(normalized.operation_id.capacity() < 1024);
}

struct Reentrant {
    shared: Weak<Shared>,
    calls: AtomicU64,
}
impl Wake for Reentrant {
    fn wake(self: Arc<Self>) {
        if let Some(shared) = self.shared.upgrade() {
            assert!(shared.state.try_lock().is_ok());
            assert!(shared.budget.usage.try_lock().is_ok());
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
    }
}
#[test]
fn progress_and_response_wakers_run_outside_queue_and_budget_locks() {
    let fixture = Fixture::new();
    let mailbox = fixture.mailbox(1);
    let wake = Arc::new(Reentrant {
        shared: Arc::downgrade(&mailbox.shared),
        calls: AtomicU64::new(0),
    });
    let waker = Waker::from(wake.clone());
    let mut cx = Context::from_waker(&waker);
    assert!(mailbox.poll_next(&mut cx).is_pending());
    let requester = mailbox.requester();
    let (_a, invocation) = fixture.invocation("principal");
    let mut future = requester.execute(invocation, CancellationToken::new());
    assert!(future.as_mut().poll(&mut cx).is_pending());
    assert_eq!(wake.calls.load(Ordering::Relaxed), 1);
    let job = next(&mailbox);
    job.complete(Ok(result()));
    assert_eq!(wake.calls.load(Ordering::Relaxed), 2);
    assert!(future.as_mut().poll(&mut cx).is_ready());
}
