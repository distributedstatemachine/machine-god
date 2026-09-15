use super::*;
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, Engine, ModelEvent, PreparedToolCall, SessionId,
    SessionIncarnationId, StopReason, Tool, ToolCall, ToolCallId, ToolError, ToolExecution,
    ToolName, ToolOutput, ToolSpec,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::atomic::AtomicU64};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    path: PathBuf,
    workspace: NativeWorkspaceAuthority,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "machine-god-principal-{}-{}",
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
        Self { path, workspace }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}
fn policy() -> NativePermissionPolicySnapshot {
    NativePermissionPolicySnapshot::new(
        crate::PermissionMode::Ask,
        Arc::new(crate::NativeConfiguredPermissionRules::default()),
    )
}
fn preferences() -> NativeModelPreferences {
    NativeModelPreferences::new(
        "selected-model",
        crate::NativeReasoningEffort::parse("high").unwrap(),
        false,
    )
    .unwrap()
}
fn registry(limit: usize) -> NativePrincipalRegistry {
    NativePrincipalRegistry::new(limit, Arc::new(NativeUndoBudget::default())).unwrap()
}
fn session(engine: &Engine) -> Session {
    engine
        .create_session(
            SessionId::new("same-id").unwrap(),
            SessionIncarnationId::new("same-incarnation").unwrap(),
        )
        .unwrap()
}
fn engine() -> Engine {
    Engine::builder()
        .provider(ScriptedModelProvider::new("test", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .session_store(InMemorySessionStore::default())
        .build()
        .unwrap()
}

#[test]
fn actual_session_identity_not_public_ids_and_registration_is_before_provider_poll() {
    let fixture = Fixture::new();
    let registry = registry(2);
    let a_engine = engine();
    let b_engine = engine();
    let a = session(&a_engine);
    let b = session(&b_engine);
    let pa = registry.register(&a, 1, &fixture.workspace).unwrap();
    let pb = registry.register(&b, 2, &fixture.workspace).unwrap();
    assert!(registry.register(&a, 3, &fixture.workspace).is_err());
    let ta = block_on(a.prompt("a")).unwrap();
    let tb = block_on(b.prompt("b")).unwrap();
    assert!(pa.begin_turn(&tb, policy(), preferences(), None).is_err());
    let ra = pa.begin_turn(&ta, policy(), preferences(), None).unwrap();
    let rb = pb.begin_turn(&tb, policy(), preferences(), None).unwrap();
    assert!(a.owns_turn(ra.witness()));
    assert!(b.owns_turn(rb.witness()));
    assert!(pa.begin_turn(&ta, policy(), preferences(), None).is_err());
    assert!(pa.requester().upgrade(&b.witness(), 1).is_err());
    assert!(pa.requester().upgrade(&a.witness(), 2).is_err());
    pa.retire();
    assert!(pa.requester().upgrade(&a.witness(), 1).is_err());
    assert!(pb.requester().upgrade(&b.witness(), 2).is_ok());
    let replacement = registry.register(&a, 3, &fixture.workspace).unwrap();
    pa.retire();
    assert!(replacement.requester().upgrade(&a.witness(), 3).is_ok());
}

#[test]
fn resident_capacity_reuses_retired_routes_and_weak_edges_do_not_retain_sessions() {
    let fixture = Fixture::new();
    let registry = registry(1);
    let engine = engine();
    let session = session(&engine);
    assert!(NativePrincipalRegistry::new(0, Arc::new(NativeUndoBudget::default())).is_err());
    assert!(NativePrincipalRegistry::new(65, Arc::new(NativeUndoBudget::default())).is_err());
    assert!(registry.register(&session, 0, &fixture.workspace).is_err());
    for generation in 1..100 {
        let principal = registry
            .register(&session, generation, &fixture.workspace)
            .unwrap();
        assert!(
            registry
                .register(&session, generation + 1, &fixture.workspace)
                .is_err()
        );
        principal.retire();
    }
    let principal = registry
        .register(&session, 100, &fixture.workspace)
        .unwrap();
    let witness = session.witness();
    let requester = principal.requester();
    drop(session);
    drop(engine);
    assert!(!witness.is_live());
    assert!(requester.upgrade(&witness, 100).is_err());
    let replacement_engine = super::tests::engine();
    let replacement_session = super::tests::session(&replacement_engine);
    assert!(
        registry
            .register(&replacement_session, 1, &fixture.workspace)
            .is_ok()
    );
    let reverse = registry.requester();
    drop(registry);
    assert!(reverse.0.upgrade().is_none());
    assert!(!principal.live());
}

#[test]
fn workspace_and_undo_selection_are_distinct_under_one_aggregate_budget() {
    let fixture = Fixture::new();
    let registry = registry(2);
    let a_engine = engine();
    let b_engine = engine();
    let a = session(&a_engine);
    let b = session(&b_engine);
    let pa = registry.register(&a, 1, &fixture.workspace).unwrap();
    let pb = registry.register(&b, 2, &fixture.workspace).unwrap();
    pa.workspace()
        .install(pa.workspace().prepare_blocking(vec![], false).unwrap())
        .unwrap();
    assert_eq!(pa.workspace().snapshot().unwrap().generation(), 1);
    assert_eq!(pb.workspace().snapshot().unwrap().generation(), 0);
    assert_eq!(fixture.workspace.snapshot().unwrap().generation(), 0);
    assert!(!Arc::ptr_eq(pa.undo(), pb.undo()));
    assert!(
        pa.undo()
            .check_principal(pa.owner(), pa.generation())
            .is_ok()
    );
    assert!(
        pa.undo()
            .check_principal(pb.owner(), pb.generation())
            .is_err()
    );
    assert_eq!(pa.undo().budget_usage(), pb.undo().budget_usage());
}

#[test]
fn stale_turn_guard_drop_cannot_invalidate_a_new_turn_registration() {
    let fixture = Fixture::new();
    let registry = registry(1);
    let engine = engine();
    let session = session(&engine);
    let principal = registry.register(&session, 1, &fixture.workspace).unwrap();
    let first = block_on(session.prompt("first")).unwrap();
    let old_guard = principal
        .begin_turn(&first, policy(), preferences(), None)
        .unwrap();
    drop(first);
    let second = block_on(session.prompt("second")).unwrap();
    let new_guard = principal
        .begin_turn(&second, policy(), preferences(), None)
        .unwrap();
    drop(old_guard);
    let active = principal.active.lock().unwrap();
    assert!(
        active
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap()
            .witness
            .same_turn(new_guard.witness())
    );
    assert!(new_guard.witness().is_live());
}

struct Probe {
    requester: NativePrincipalRequester,
    seen: Arc<Mutex<Vec<NativeManagedCallLease>>>,
}

#[test]
fn run_quota_and_registration_liveness_are_distinct_from_retained_resource_custody() {
    use super::super::scheduler::{ManagedScheduler, SchedulerLimits};
    let fixture = Fixture::new();
    let registry = registry(1);
    let engine = engine();
    let session = session(&engine);
    let principal = registry.register(&session, 1, &fixture.workspace).unwrap();
    let turn = block_on(session.prompt("work")).unwrap();
    let scheduler = ManagedScheduler::new(SchedulerLimits::default());
    let resident = scheduler.reserve_resident().unwrap();
    let (run, settlement) = scheduler
        .register_run(&resident, std::num::NonZeroU64::new(7).unwrap(), &turn)
        .unwrap();
    let guard = principal
        .begin_turn(&turn, policy(), preferences(), Some(run.reference()))
        .unwrap();
    // A lifecycle-only internal lease: authentication itself is exercised by
    // the real engine-driven Probe below, not manufactured by this fixture.
    let lease = NativeManagedCallLease {
        state: guard.state.clone(),
    };
    assert!(!lease.is_live());
    block_on(run.acquire()).unwrap();
    assert!(lease.is_live());
    assert_eq!(lease.run().unwrap().work_generation().unwrap().get(), 7);
    drop(guard);
    assert!(!lease.is_live());
    assert!(turn.witness().is_live());
    assert!(
        lease
            .principal()
            .undo()
            .check_principal(principal.owner(), 1)
            .is_ok()
    );
    run.finish();
    drop(turn);
    settlement.complete().unwrap();
}
impl Tool for Probe {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("probe").unwrap(),
            description: "fixture".into(),
            input_schema: json!({}),
        }
    }
    fn prepare(&self, call: ToolCall) -> std::result::Result<PreparedToolCall, ToolError> {
        Ok(PreparedToolCall::without_authority(call.arguments))
    }
    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<ToolOutput, ToolError>> {
        panic!("structural path must not authenticate")
    }
    fn execute_admitted(
        &self,
        invocation: AdmittedToolInvocation,
        _: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<ToolExecution, ToolError>> {
        Box::pin(async move {
            let lease = self.requester.claim_tool(&invocation).unwrap();
            assert!(lease.is_live());
            assert!(self.requester.claim_tool(&invocation).is_err());
            self.seen.lock().unwrap().push(lease);
            Ok(ToolExecution::output(ToolOutput::success(
                json!({"ok":true}),
            )))
        })
    }
}
#[test]
fn actual_calls_claim_once_repeated_provider_ids_are_fresh_and_settlement_does_not_restore_authority()
 {
    let fixture = Fixture::new();
    let registry = registry(2);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut steps = Vec::new();
    for _ in 0..2 {
        steps.push(ModelProviderStep::events([
            ModelEvent::ToolCall {
                call: ToolCall {
                    id: ToolCallId::new("repeated").unwrap(),
                    name: ToolName::new("probe").unwrap(),
                    arguments: json!({}),
                },
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ]));
        steps.push(ModelProviderStep::events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }]));
    }
    let provider = ScriptedModelProvider::new("test", steps);
    // A different actual session with identical public IDs is an earlier
    // routing candidate, never an authority match for this engine's calls.
    let decoy_engine = engine();
    let decoy_session = session(&decoy_engine);
    let decoy = registry
        .register(&decoy_session, 99, &fixture.workspace)
        .unwrap();
    let decoy_turn = block_on(decoy_session.prompt("unpolled")).unwrap();
    let _decoy_registration = decoy
        .begin_turn(&decoy_turn, policy(), preferences(), None)
        .unwrap();
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(InMemorySessionStore::default())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .tool(Probe {
            requester: registry.requester(),
            seen: seen.clone(),
        })
        .build()
        .unwrap();
    let session = session(&engine);
    let principal = registry.register(&session, 7, &fixture.workspace).unwrap();
    for count in 1..=2 {
        let turn = block_on(session.prompt("call")).unwrap();
        let guard = principal
            .begin_turn(&turn, policy(), preferences(), None)
            .unwrap();
        assert!(guard.witness().is_live());
        principal
            .workspace()
            .install(
                principal
                    .workspace()
                    .prepare_blocking(vec![], false)
                    .unwrap(),
            )
            .unwrap();
        let events = block_on(turn.collect::<Vec<_>>());
        assert!(events.iter().all(std::result::Result::is_ok), "{events:?}");
        assert_eq!(seen.lock().unwrap().len(), count);
        assert!(!seen.lock().unwrap().last().unwrap().is_live());
        assert_eq!(
            seen.lock().unwrap().last().unwrap().preferences(),
            &preferences()
        );
        assert_eq!(
            seen.lock()
                .unwrap()
                .last()
                .unwrap()
                .workspace()
                .generation(),
            (count - 1) as u64
        );
        drop(guard);
    }
    let witness = session.witness();
    principal.retire();
    drop(session);
    drop(engine);
    assert!(!witness.is_live());
    let leases = seen.lock().unwrap();
    assert!(leases.iter().all(|lease| !lease.is_live()));
    assert!(
        leases
            .iter()
            .all(|lease| lease.principal().generation() == 7)
    );
    assert!(!provider.requests().is_empty());
}
