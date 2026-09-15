use super::*;
use machine_god_core::{
    ManagedFailureCode, ManagedResultStatus, ManagedSubagentCommand, ManagedSubagentError,
    ManagedSubagentResult,
};

fn command(value: serde_json::Value) -> ManagedSubagentCommand {
    ManagedSubagentCommand::decode(serde_json::Value::Object(serde_json::Map::from_iter([(
        "command".into(),
        value,
    )])))
    .unwrap()
}

fn create() -> ManagedSubagentCommand {
    command(serde_json::json!({"create":{"name":"human-worker","mode":"persistent"}}))
}

async fn open(
    fixture: &mut Fixture,
) -> (NativeInteractiveSession, crate::NativeOwnedWorkerCompletion) {
    let (mut startup, host) = preselection::prepare(fixture).await;
    let completion = host.terminal_shutdown_completion().unwrap();
    startup
        .request_open(NativeInteractiveInitialSession::Fresh, 1)
        .unwrap();
    let owner = poll_fn(|cx| startup.poll_open(cx, 1))
        .await
        .unwrap()
        .unwrap();
    drop(startup);
    drop(host);
    (owner, completion)
}

async fn response(
    owner: &mut NativeInteractiveSession,
    mut response: crate::NativeManagedCommandResponse,
) -> Result<ManagedSubagentResult, ManagedSubagentError> {
    poll_fn(|cx| {
        let progress = owner.poll_progress(cx, 10);
        assert!(
            owner.managed_error().is_none(),
            "{:?}",
            owner.managed_error()
        );
        assert!(
            owner.shutdown_error().is_none(),
            "{:?}",
            owner.shutdown_error()
        );
        let _ = owner.take_presentation();
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        response.as_mut().poll(cx)
    })
    .await
}

async fn submit(
    owner: &mut NativeInteractiveSession,
    command: ManagedSubagentCommand,
) -> ManagedSubagentResult {
    let pending = owner
        .request_managed_command(command, CancellationToken::new())
        .unwrap();
    response(owner, pending).await.unwrap()
}

async fn close(
    mut owner: NativeInteractiveSession,
    completion: crate::NativeOwnedWorkerCompletion,
) {
    owner.request_shutdown();
    while !owner.is_closed() {
        let _ = outcome(&mut owner).await;
    }
    drop(owner);
    completion.wait().await;
}

#[test]
fn human_commands_reach_retained_child_after_parent_replacement_without_a_model_call() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let created = submit(&mut owner, create()).await;
        assert_eq!(created.status, ManagedResultStatus::Created, "{created:?}");
        let child = created.child_id.unwrap();
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
        owner
            .request_transition(NativeInteractiveTransition::New, 11)
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Transition(_)
        ));
        let inspected = submit(
            &mut owner,
            command(serde_json::json!({
                "inspect":{"id":child,"sections":["status"]}
            })),
        )
        .await;
        assert!(inspected.ok, "{inspected:?}");
        let sent = submit(
            &mut owner,
            command(serde_json::json!({
                "message":{"send":{"id":child,"content":"standalone user request"}}
            })),
        )
        .await;
        assert_eq!(sent.status, ManagedResultStatus::MessageQueued, "{sent:?}");
        poll_fn(|cx| {
            let progress = owner.poll_progress(cx, 20);
            assert!(owner.managed_error().is_none());
            if fixture.transport.requests.lock().unwrap().len() == 1
                && owner
                    .managed_agents()
                    .first()
                    .is_some_and(|agent| agent.state == machine_god_core::ManagedAgentState::Idle)
            {
                return Poll::Ready(());
            }
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        })
        .await;
        close(owner, completion).await;
    });
    assert_eq!(fixture.transport.requests.lock().unwrap().len(), 1);
}

#[test]
fn human_commands_cannot_relax_the_captured_permission_policy() {
    let mut fixture = Fixture::with_options("ask", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let denied = submit(
            &mut owner,
            command(serde_json::json!({
                "create":{"name":"too-permissive","mode":"persistent","permission_mode":"yolo"}
            })),
        )
        .await;
        assert_eq!(
            denied.error_code,
            Some(ManagedFailureCode::PermissionDenied)
        );
        assert!(owner.managed_agents().is_empty());
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn quiescence_rejects_queued_human_commands_and_releases_original_permit() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let pending = owner
            .request_managed_command(create(), CancellationToken::new())
            .unwrap();
        let mut guard = owner.runtime().begin_quiescence().unwrap();
        let rejected = response(&mut owner, pending).await.unwrap();
        assert_eq!(
            rejected.error_code,
            Some(ManagedFailureCode::CallerUnavailable)
        );
        guard.wait_idle().await.unwrap();
        drop(guard);
        assert!(owner.managed_agents().is_empty());
        assert!(submit(&mut owner, create()).await.ok);
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn explicit_human_relationships_keep_graph_checks_without_model_approval() {
    let mut fixture = Fixture::with_options("ask", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let first = submit(&mut owner, create()).await.child_id.unwrap();
        let second = submit(&mut owner, create()).await.child_id.unwrap();
        let reparented = submit(
            &mut owner,
            command(serde_json::json!({
                "relationship":{"action":"reparent","id":first,"parent_id":second}
            })),
        )
        .await;
        assert_eq!(
            reparented.status,
            ManagedResultStatus::RelationshipChanged,
            "{reparented:?}"
        );
        let cycle = submit(
            &mut owner,
            command(serde_json::json!({
                "relationship":{"action":"reparent","id":second,"parent_id":first}
            })),
        )
        .await;
        assert_eq!(
            cycle.error_code,
            Some(ManagedFailureCode::RelationshipCycle)
        );
        let detached = submit(
            &mut owner,
            command(serde_json::json!({
                "relationship":{"action":"detach","id":first}
            })),
        )
        .await;
        assert_eq!(detached.status, ManagedResultStatus::RelationshipChanged);
        assert_eq!(fixture.prompt.calls.load(Ordering::Relaxed), 0);
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

async fn waiting(
    owner: &mut NativeInteractiveSession,
    child: &str,
    cancellation: CancellationToken,
) -> crate::NativeManagedCommandResponse {
    let pending = owner
        .request_managed_command(
            command(serde_json::json!({
                "inspect":{"id":child,"sections":["status"],
                    "wait":{"until":"settled","after_generation":1,"timeout_ms":30000}}
            })),
            cancellation,
        )
        .unwrap();
    poll_fn(|cx| {
        let progress = owner.poll_progress(cx, 10);
        assert!(owner.managed_error().is_none());
        if owner.managed_progress().unwrap().waiters == 1 {
            return Poll::Ready(());
        }
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    })
    .await;
    pending
}

#[test]
fn human_inspection_wait_is_woken_by_cancellation_without_deadline_or_model_run() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        let cancellation = CancellationToken::new();
        let pending = waiting(&mut owner, &child, cancellation.clone()).await;
        let (result, ()) = futures_util::future::join(response(&mut owner, pending), async {
            tokio::task::yield_now().await;
            cancellation.cancel();
        })
        .await;
        assert_eq!(result.unwrap().status, ManagedResultStatus::Rejected);
        assert_eq!(owner.managed_progress().unwrap().waiters, 0);
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn parent_transition_settles_an_already_registered_human_wait() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        let pending = waiting(&mut owner, &child, CancellationToken::new()).await;
        owner
            .request_transition(NativeInteractiveTransition::New, 11)
            .unwrap();
        let rejected = response(&mut owner, pending).await.unwrap();
        assert_eq!(rejected.status, ManagedResultStatus::Rejected);
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Transition(_)
        ));
        assert_eq!(owner.managed_progress().unwrap().waiters, 0);
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn human_queue_is_bounded_and_retained_responses_do_not_retain_shutdown_resources() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let mut pending = Vec::new();
        for _ in 0..64 {
            pending.push(
                owner
                    .request_managed_command(create(), CancellationToken::new())
                    .unwrap(),
            );
        }
        assert!(matches!(
            owner.request_managed_command(create(), CancellationToken::new()),
            Err(ManagedSubagentError::ResourceLimit)
        ));
        close(owner, completion).await;
        for response in pending {
            assert!(matches!(
                response.await,
                Err(ManagedSubagentError::Unavailable)
            ));
        }
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}
