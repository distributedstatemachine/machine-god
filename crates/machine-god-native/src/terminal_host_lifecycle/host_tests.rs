use super::*;
use futures_executor::block_on;
use machine_god_core::TerminalLifecycle;
use std::task::{Context, Waker};

fn owner(context: &ToolContext) -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        context.session_id.clone(),
        context.session_incarnation_id.clone(),
    )
}
fn next_context(name: &str) -> ToolContext {
    let mut context = Fixture::context();
    context.session_id = SessionId::new(name).unwrap();
    context.session_incarnation_id = SessionIncarnationId::new(name).unwrap();
    context
}
fn requester(fixture: &Fixture) -> NativeTerminalLifecycleRequester {
    fixture.resource.as_ref().unwrap().lifecycle_requester()
}
fn start(fixture: &Fixture) -> TerminalSessionId {
    let TerminalActionResult::Start { session, .. } = fixture.action(json!({
        "action":"start", "command":"sleep 120", "profile":"clean"
    })) else {
        panic!("start receipt")
    };
    session.session_id
}

#[test]
fn lifecycle_futures_are_inert_and_requester_does_not_keep_host_alive() {
    let mut fixture = Fixture::new();
    let lifecycle = requester(&fixture);
    drop(lifecycle.handoff(
        owner(&fixture.context),
        owner(&next_context("next")),
        CancellationToken::new(),
    ));
    drop(lifecycle.reset_current_workspace(owner(&fixture.context), CancellationToken::new()));
    drop(lifecycle.activate_session(owner(&fixture.context), CancellationToken::new()));
    assert!(!fixture.root.join("state/terminal-v1").exists());
    drop(fixture.resource.take());
    assert_eq!(
        block_on(lifecycle.activate_session(owner(&fixture.context), CancellationToken::new())),
        Err(NativeTerminalTransitionError::Closed)
    );
    assert!(!fixture.root.join("state/terminal-v1").exists());
}

#[test]
fn lifecycle_handoff_keeps_live_process_and_routes_a_b_c_without_relabeling_history() {
    let mut fixture = Fixture::new();
    let lifecycle = requester(&fixture);
    let a = fixture.context.clone();
    let id = start(&fixture);
    let original_namespace = crate::terminal_catalog::owner_name(
        fixture.root.join("workspace").to_str().unwrap(),
        &owner(&a),
    );
    let b = next_context("b");
    let c = next_context("c");
    assert_eq!(
        block_on(lifecycle.handoff(owner(&a), owner(&b), CancellationToken::new()))
            .unwrap()
            .transferred(),
        1
    );
    assert!(
        fixture
            .try_action(json!({"action":"inspect","session_id":id.as_str()}))
            .is_err()
    );
    fixture.context = b.clone();
    let TerminalActionResult::List { sessions } =
        fixture.action(json!({"action":"list","task_id":"b"}))
    else {
        panic!("list")
    };
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].lifecycle, TerminalLifecycle::Running);
    assert_eq!(
        block_on(lifecycle.handoff(owner(&b), owner(&c), CancellationToken::new()))
            .unwrap()
            .transferred(),
        1
    );
    assert!(
        fixture
            .try_action(json!({"action":"inspect","session_id":id.as_str()}))
            .is_err()
    );
    fixture.context = c;
    fixture.close(&id);
    // Explicit reactivation does not recover the original-owner fallback.
    block_on(lifecycle.activate_session(owner(&a), CancellationToken::new())).unwrap();
    fixture.context = a.clone();
    assert!(
        fixture
            .try_action(json!({"action":"inspect","session_id":id.as_str()}))
            .is_err()
    );
    let TerminalActionResult::List { sessions } = fixture.action(json!({"action":"list"})) else {
        panic!("list")
    };
    assert!(sessions.is_empty());
    // All persisted artifacts remain in A's original namespace.
    let terminal_root = fixture.root.join("state/terminal-v1");
    assert!(terminal_root.join(original_namespace).exists());
}

#[test]
fn lifecycle_handoff_preserves_terminated_history_and_reset_forgets_only_its_routes() {
    let mut fixture = Fixture::new();
    let lifecycle = requester(&fixture);
    let a = fixture.context.clone();
    let id = start(&fixture);
    fixture.close(&id);
    let b = next_context("b");
    block_on(lifecycle.handoff(owner(&a), owner(&b), CancellationToken::new())).unwrap();
    fixture.context = b.clone();
    let TerminalActionResult::Inspect { session, .. } =
        fixture.action(json!({"action":"inspect","session_id":id.as_str()}))
    else {
        panic!("inspect")
    };
    assert_eq!(session.lifecycle, TerminalLifecycle::Closed);
    let receipt =
        block_on(lifecycle.reset_current_workspace(owner(&b), CancellationToken::new())).unwrap();
    assert_eq!(receipt.entries().len(), 1);
    assert_eq!(receipt.entries()[0].id(), &id);
    assert_eq!(
        receipt.entries()[0].outcome(),
        NativeTerminalResetOutcome::AlreadyTerminatedForgotten
    );
    block_on(lifecycle.activate_session(owner(&b), CancellationToken::new())).unwrap();
    assert!(
        fixture
            .try_action(json!({"action":"inspect","session_id":id.as_str()}))
            .is_err()
    );
}

#[test]
fn lifecycle_pre_cancelled_handoff_preserves_source_access() {
    let fixture = Fixture::new();
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        block_on(requester(&fixture).handoff(
            owner(&fixture.context),
            owner(&next_context("b")),
            token
        )),
        Err(NativeTerminalTransitionError::Cancelled)
    ));
    fixture.action(json!({"action":"list"}));
}

#[test]
fn lifecycle_reset_retains_a_resource_pinned_by_an_unconsumed_receipt() {
    let mut fixture = Fixture::new();
    let lifecycle = requester(&fixture);
    let a = fixture.context.clone();
    let id = start(&fixture);
    let storage = owner(&a);
    let session = id.clone();
    let lease = block_on(
        lifecycle
            .requester
            .request_with_context(CancellationToken::new(), move |context| {
                context.registry.lease(&storage, &session)
            }),
    )
    .unwrap()
    .unwrap();
    fixture.close(&id);
    let receipt =
        block_on(lifecycle.reset_current_workspace(owner(&a), CancellationToken::new())).unwrap();
    assert_eq!(
        receipt.entries()[0].outcome(),
        NativeTerminalResetOutcome::RetainedIndeterminate
    );
    let b = next_context("b");
    block_on(lifecycle.handoff(owner(&a), owner(&b), CancellationToken::new())).unwrap();
    fixture.context = b.clone();
    fixture.action(json!({"action":"inspect","session_id":id.as_str()}));
    drop(lease);
    assert_eq!(
        block_on(lifecycle.reset_current_workspace(owner(&b), CancellationToken::new()))
            .unwrap()
            .entries()[0]
            .outcome(),
        NativeTerminalResetOutcome::AlreadyTerminatedForgotten
    );
}

#[test]
fn lifecycle_dropped_committed_receipt_does_not_rollback_the_route() {
    let mut fixture = Fixture::new();
    let lifecycle = requester(&fixture);
    fixture.action(json!({"action":"list"}));
    let b = next_context("b");
    let mut transition =
        lifecycle.handoff(owner(&fixture.context), owner(&b), CancellationToken::new());
    let _ = transition
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()));
    // This second owner request settles after the queued handoff. Deliberately
    // discard its unconsumed receipt: publication is not a caller lifetime.
    block_on(
        lifecycle
            .requester
            .request_with_context(CancellationToken::new(), |_| ()),
    )
    .unwrap();
    drop(transition);
    assert!(fixture.try_action(json!({"action":"list"})).is_err());
    fixture.context = b;
    fixture.action(json!({"action":"list"}));
}

#[test]
fn lifecycle_old_generation_is_rejected_even_after_explicit_reactivation() {
    let fixture = Fixture::new();
    let lifecycle = requester(&fixture);
    let a = owner(&fixture.context);
    let original = a.clone();
    let (old_access, _) = block_on(
        lifecycle
            .requester
            .request_with_context(CancellationToken::new(), move |context| {
                context.state.access.acquire(original)
            }),
    )
    .unwrap()
    .unwrap();
    block_on(lifecycle.handoff(
        a.clone(),
        owner(&next_context("b")),
        CancellationToken::new(),
    ))
    .unwrap();
    block_on(lifecycle.activate_session(a, CancellationToken::new())).unwrap();
    assert!(
        block_on(terminal_host_dispatch::dispatch_with_access(
            lifecycle.requester.clone(),
            resident_authority(fixture.context.clone()),
            machine_god_core::TerminalActionRequest::List {
                filters: machine_god_core::TerminalListFilters::default()
            },
            TerminalMonitorActivation::default(),
            CancellationToken::new(),
            Some(old_access)
        ))
        .is_err()
    );
    fixture.action(json!({"action":"list"}));
}

#[test]
fn lifecycle_cold_transferred_history_remains_readable_and_can_be_forgotten() {
    let mut fixture = Fixture::new();
    let lifecycle = requester(&fixture);
    let a = owner(&fixture.context);
    let id = start(&fixture);
    fixture.close(&id);
    let b = next_context("b");
    block_on(lifecycle.handoff(a.clone(), owner(&b), CancellationToken::new())).unwrap();
    let released = id.clone();
    block_on(
        lifecycle
            .requester
            .request_with_context(CancellationToken::new(), move |context| {
                context.registry.release(&a, &released)
            }),
    )
    .unwrap()
    .unwrap();
    fixture.context = b.clone();
    let TerminalActionResult::Inspect { session, .. } =
        fixture.action(json!({"action":"inspect", "session_id":id.as_str()}))
    else {
        panic!("inspect")
    };
    assert_eq!(session.lifecycle, TerminalLifecycle::Closed);
    let receipt =
        block_on(lifecycle.reset_current_workspace(owner(&b), CancellationToken::new())).unwrap();
    assert_eq!(
        receipt.entries()[0].outcome(),
        NativeTerminalResetOutcome::AlreadyTerminatedForgotten
    );
}

#[test]
fn lifecycle_handoff_cancels_wait_without_revoking_the_destination_writer() {
    let mut fixture = Fixture::new();
    let lifecycle = requester(&fixture);
    let id = start(&fixture);
    fixture.action(json!({"action":"write", "session_id":id.as_str(), "lease":"acquire"}));
    let mut wait = fixture.future(json!({"action":"wait", "session_id":id.as_str(), "return_when":{"kind":"match","pattern":"never"},"wait_ceiling_ms":5000}), CancellationToken::new());
    // Poll through access acquisition until the terminal wait is admitted.
    for _ in 0..3 {
        assert!(
            wait.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        block_on(
            lifecycle
                .requester
                .request_with_context(CancellationToken::new(), |_| ()),
        )
        .unwrap();
    }
    let b = next_context("b");
    block_on(lifecycle.handoff(owner(&fixture.context), owner(&b), CancellationToken::new()))
        .unwrap();
    let TerminalActionResult::Wait { outcome, .. } = block_on(wait).unwrap() else {
        panic!("wait")
    };
    assert_eq!(
        outcome,
        machine_god_core::TerminalReturnOutcome::Cancelled {}
    );
    fixture.context = b;
    fixture.action(json!({"action":"write", "session_id":id.as_str(), "lease":"acquire"}));
    fixture.close(&id);
}

#[test]
fn lifecycle_reset_stops_only_the_selected_principals_live_process() {
    let mut fixture = Fixture::new();
    let lifecycle = requester(&fixture);
    let a = owner(&fixture.context);
    let first = start(&fixture);
    fixture.context = next_context("other");
    let second = start(&fixture);
    let receipt = block_on(lifecycle.reset_current_workspace(a, CancellationToken::new())).unwrap();
    assert_eq!(receipt.entries().len(), 1);
    assert_eq!(receipt.entries()[0].id(), &first);
    assert_eq!(
        receipt.entries()[0].outcome(),
        NativeTerminalResetOutcome::StoppedAndForgotten
    );
    let TerminalActionResult::Inspect { session, .. } =
        fixture.action(json!({"action":"inspect","session_id":second.as_str()}))
    else {
        panic!("inspect")
    };
    assert_eq!(session.lifecycle, TerminalLifecycle::Running);
    fixture.close(&second);
}

#[test]
fn lifecycle_locked_publication_preserves_failed_handoff_and_retains_reset_authority() {
    use rustix::fs::{FlockOperation, flock};
    let fixture = Fixture::new();
    let lifecycle = requester(&fixture);
    let a = owner(&fixture.context);
    let id = start(&fixture);
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.root.join("state/terminal-v1/profile-lock"))
        .unwrap();
    flock(&lock, FlockOperation::LockExclusive).unwrap();
    let handoff = block_on(lifecycle.handoff(
        a.clone(),
        owner(&next_context("b")),
        CancellationToken::new(),
    ));
    assert!(matches!(
        handoff,
        Err(NativeTerminalTransitionError::Preparation)
    ));
    let receipt =
        block_on(lifecycle.reset_current_workspace(a.clone(), CancellationToken::new())).unwrap();
    assert_eq!(
        receipt.entries()[0].outcome(),
        NativeTerminalResetOutcome::RetainedIndeterminate
    );
    flock(&lock, FlockOperation::Unlock).unwrap();
    block_on(lifecycle.activate_session(a, CancellationToken::new())).unwrap();
    let TerminalActionResult::Inspect { session, .. } =
        fixture.action(json!({"action":"inspect","session_id":id.as_str()}))
    else {
        panic!("inspect")
    };
    assert_eq!(session.lifecycle, TerminalLifecycle::Running);
    fixture.close(&id);
}
